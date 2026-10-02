#[cfg(test)]
use runner_backend::model::Runtime;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::bail;
use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, Div, FontWeight, KeyDownEvent, PathPromptOptions,
    ScrollHandle, SharedString, Window,
};
use runner_backend::model::{CodexSpeed, Role};
use runner_backend::ops::project::ProjectScope;
use runner_backend::ops::runtime::{
    filter_selectable_runtime_catalog, RuntimeCatalogEntry, RuntimeCatalogOption,
};

use runner_app::ui::{
    effective_working_dir, working_dir_placeholder, working_dir_text_field, Button, ButtonVariant,
    Field, IconButton, Modal, ModelField, OverlayWidth, RoleAvatar, Scrollbar, SelectHandler,
    SelectLeading, SelectOption, StyledSelect, TextField, WorkingDirField,
};

use super::profile_page::{column_text, section_label, text_action};
use super::roles::logic::role_setting_label;
use super::*;
use crate::chat_icon::runtime_mark;
use crate::*;

const START_CHAT_MODE_FILE: &str = "start-chat-mode";
const MODAL_WIDTH: f32 = 560.;
const CARD_PADDING: f32 = 12.;
const COLUMN_GAP: f32 = 12.;
const EFFORT_COLUMN_WIDTH: f32 = 160.;
const SPEED_COLUMN_WIDTH: f32 = 104.;
const ROLE_HINT: &str = "Starts with the role's settings. Change any of them for this chat only.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChatMode {
    Role,
    Runtime,
}

impl ChatMode {
    fn from_persisted(value: Option<&str>) -> Self {
        if value.is_some_and(|value| matches!(value.trim(), "role" | "runner")) {
            Self::Role
        } else {
            Self::Runtime
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Clone)]
enum ChatTarget {
    NewTab,
    Pane { tab_id: String, pane_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewTerminalTarget {
    MissionDrawer,
    ChatDrawer,
    Tab,
}

fn new_terminal_target(
    route: &AppRoute,
    mission_drawer_available: bool,
    active_tab_is_terminal: bool,
) -> NewTerminalTarget {
    if matches!(route, AppRoute::Mission(_)) && mission_drawer_available {
        NewTerminalTarget::MissionDrawer
    } else if !active_tab_is_terminal {
        NewTerminalTarget::ChatDrawer
    } else {
        NewTerminalTarget::Tab
    }
}

fn new_terminal_empty_pane(layout: &PaneLayout) -> Option<String> {
    let leaves = layout.root.leaves();
    leaves
        .iter()
        .find(|leaf| leaf.id == layout.focused_pane_id && leaf.session_id.is_none())
        .or_else(|| leaves.iter().find(|leaf| leaf.session_id.is_none()))
        .map(|leaf| leaf.id.clone())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartChatSelection {
    Role,
    RoleRuntime,
    Runtime,
    Effort,
    Speed,
}

pub(crate) struct StartChatModal {
    target: ChatTarget,
    scope: ProjectScope,
    mode: ChatMode,
    roles: Vec<Role>,
    runtimes: Vec<RuntimeCatalogEntry>,
    role_id: Option<String>,
    runtime_name: Option<String>,
    /// The agent a role chat runs on when it is not the role's own; picking
    /// the role's own agent clears it.
    role_runtime_override: Option<String>,
    /// The Effort and Speed the controls show; `baseline_effort` and
    /// `baseline_speed` are what leaves them untouched.
    effort: String,
    speed: String,
    title: Entity<TextField>,
    cwd: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    role_select: Entity<StyledSelect>,
    role_runtime_select: Entity<StyledSelect>,
    runtime_select: Entity<StyledSelect>,
    effort_select: Entity<StyledSelect>,
    speed_select: Entity<StyledSelect>,
    scroll_handle: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    role_mode_focus: FocusHandle,
    direct_mode_focus: FocusHandle,
    /// One Reset per role control, in `ResetKind` order.
    reset_focus: [FocusHandle; 4],
    browse_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    agents_checking: bool,
    agents_error: Option<String>,
    submitting: bool,
    error: Option<String>,
    _model_subscription: gpui::Subscription,
}

impl StartChatModal {
    fn mode_focus(&self) -> FocusHandle {
        match self.mode {
            ChatMode::Role => self.role_mode_focus.clone(),
            ChatMode::Runtime => self.direct_mode_focus.clone(),
        }
    }

    fn picker_focus(&self, cx: &App) -> FocusHandle {
        match self.mode {
            ChatMode::Role if !self.roles.is_empty() => self.role_select.read(cx).focus_handle(),
            ChatMode::Runtime if !self.runtimes.is_empty() => {
                self.runtime_select.read(cx).focus_handle()
            }
            _ => self.title.read(cx).focus_handle(),
        }
    }

    fn selected_role(&self) -> Option<&Role> {
        self.role_id
            .as_deref()
            .and_then(|role_id| self.roles.iter().find(|role| role.id == role_id))
    }

    fn selected_runtime(&self) -> Option<&RuntimeCatalogEntry> {
        self.runtime_name
            .as_deref()
            .and_then(|name| self.runtime_entry(name))
    }

    fn runtime_entry(&self, name: &str) -> Option<&RuntimeCatalogEntry> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.name.key() == name)
    }

    /// The agent a role chat runs on: the override, else the role's own.
    fn role_agent(&self) -> Option<&str> {
        self.role_runtime_override
            .as_deref()
            .or_else(|| self.selected_role().map(|role| role.runtime.as_str()))
    }

    /// Whether a role chat runs on the role's own agent, where the controls
    /// start from the role's model, effort and speed.
    fn on_own_agent(&self) -> bool {
        self.role_runtime_override.is_none()
    }

    fn active_runtime(&self) -> Option<&RuntimeCatalogEntry> {
        match self.mode {
            ChatMode::Role => self.role_agent().and_then(|name| self.runtime_entry(name)),
            ChatMode::Runtime => self.selected_runtime(),
        }
    }

    fn codex_speed_visible(&self) -> bool {
        let key = match self.mode {
            ChatMode::Role => self.role_agent(),
            ChatMode::Runtime => self.runtime_name.as_deref(),
        };
        key.is_some_and(|key| {
            crate::runtime_ui::catalog_capabilities(&self.runtimes, key).codex_speed
        })
    }

    fn effective_speed(&self) -> Option<CodexSpeed> {
        self.codex_speed_visible()
            .then(|| parse_speed(&self.speed))
            .flatten()
    }

    /// The Effort that leaves the control untouched: the role's on its own
    /// agent, else the agent's default, else none known.
    fn baseline_effort(&self) -> String {
        let runtime = self.active_runtime();
        inheriting_role(self)
            .and_then(|role| trimmed(role.effort.as_deref()))
            .map(str::to_owned)
            .or_else(|| {
                let runtime = runtime?;
                runtime
                    .default_effort
                    .clone()
                    .filter(|effort| runtime.efforts.iter().any(|option| option.value == *effort))
            })
            .unwrap_or_default()
    }

    /// The Speed that leaves the control untouched: the role's on its own
    /// Codex agent, else Inherit.
    fn baseline_speed(&self) -> String {
        match inheriting_role(self)
            .filter(|role| {
                crate::runtime_ui::catalog_capabilities(&self.runtimes, &role.runtime).codex_speed
            })
            .and_then(|role| role.codex_speed)
        {
            Some(CodexSpeed::Standard) => "standard",
            Some(CodexSpeed::Fast) => "fast",
            None => "inherit",
        }
        .into()
    }

    /// The model the request carries: the one typed, unless it is the role's
    /// own, which leaves the control untouched.
    fn model_override(&self, cx: &App) -> Option<String> {
        let role_model = inheriting_role(self).and_then(|role| trimmed(role.model.as_deref()));
        normalized_value(self.model.read(cx).text())
            .filter(|text| Some(text.as_str()) != role_model)
    }

    fn effort_override(&self) -> Option<String> {
        (!self.effort.is_empty() && self.effort != self.baseline_effort())
            .then(|| self.effort.clone())
    }

    fn speed_override(&self) -> Option<CodexSpeed> {
        (self.speed != self.baseline_speed())
            .then(|| self.effective_speed())
            .flatten()
    }

    /// The role controls that differ from the role's own setup. On the role's
    /// own agent that is the model, effort and speed changed; on another agent
    /// it is the agent alone, whose defaults the rest start from.
    fn role_overrides(&self, cx: &App) -> RoleOverrides {
        let inheriting = inheriting_role(self).is_some();
        RoleOverrides {
            runtime: self.mode == ChatMode::Role
                && self.selected_role().is_some()
                && !self.on_own_agent(),
            model: inheriting && self.model_override(cx).is_some(),
            effort: inheriting && self.effort_override().is_some(),
            speed: inheriting && self.speed_override().is_some(),
        }
    }

    fn can_submit(&self) -> bool {
        !self.submitting
            && match self.mode {
                ChatMode::Role => self.selected_role().is_some(),
                ChatMode::Runtime => self.selected_runtime().is_some(),
            }
    }

    fn is_composing(&self, cx: &Context<NativeRoot>) -> bool {
        self.title.read(cx).is_composing()
            || self.cwd.read(cx).is_composing()
            || self.model.read(cx).is_composing()
    }
}

/// Which role controls carry an override, each with its amber dot and Reset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RoleOverrides {
    runtime: bool,
    model: bool,
    effort: bool,
    speed: bool,
}

/// A role control's Reset, in `StartChatModal::reset_focus` order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResetKind {
    Runtime,
    Model,
    Effort,
    Speed,
}

#[derive(Debug, Eq, PartialEq)]
enum StartRequest {
    Role {
        role_id: String,
        runtime: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<CodexSpeed>,
        cwd: Option<String>,
    },
    Runtime {
        runtime: String,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<CodexSpeed>,
        cwd: Option<String>,
    },
}

impl NativeRoot {
    pub(crate) fn create_modal_open(&self) -> bool {
        self.start_chat_modal.is_some() || self.start_mission_modal.is_some()
    }

    fn focused_empty_chat_pane(&self) -> Option<String> {
        (self.route == AppRoute::Chat)
            .then(|| self.tabs.active())
            .flatten()
            .and_then(|layout| {
                layout
                    .root
                    .leaves()
                    .into_iter()
                    .find(|leaf| leaf.id == layout.focused_pane_id && leaf.session_id.is_none())
                    .map(|leaf| leaf.id.clone())
            })
    }

    pub(crate) fn new_terminal_action(
        &mut self,
        _: &NewTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        if let Some(pane_id) = self.focused_empty_chat_pane() {
            let original = self.tabs.active().unwrap().clone();
            let (scope, cwd) = self.terminal_start_location(cx);
            self.spawn_terminal_in_pane(pane_id, original, scope, cwd, window, cx);
        } else {
            self.new_terminal_tab(
                ProjectScope::or_root(self.active_project_id(cx)),
                window,
                cx,
            );
        }
    }

    pub(crate) fn new_mission_action(
        &mut self,
        _: &NewMission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        self.open_start_mission_modal(
            None,
            ProjectScope::or_root(self.active_project_id(cx)),
            window,
            cx,
        );
    }

    fn confirm_start_chat(
        &mut self,
        _: &ConfirmStartChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self
            .start_chat_modal
            .as_ref()
            .filter(|modal| modal.can_submit() && !modal.is_composing(cx))
        else {
            return;
        };
        for select in [
            &modal.role_select,
            &modal.role_runtime_select,
            &modal.runtime_select,
            &modal.effort_select,
            &modal.speed_select,
        ] {
            select.update(cx, |select, cx| select.close(cx));
        }
        modal.model_field.update(cx, |field, cx| field.close(cx));
        self.submit_start_chat(window, cx);
    }

    fn mouse_switch_start_chat_mode(
        &mut self,
        mode: ChatMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = window.focused(cx);
        self.set_start_chat_mode(mode, cx);
        if let Some(modal) = &self.start_chat_modal {
            let survives = focused.as_ref().is_some_and(|focus| {
                *focus == modal.direct_mode_focus
                    || *focus == modal.role_mode_focus
                    || start_chat_focus_order(modal, cx).contains(focus)
            });
            if !survives {
                modal.picker_focus(cx).focus(window, cx);
            }
        }
    }

    fn switch_start_chat_mode(
        &mut self,
        mode: ChatMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .start_chat_modal
            .as_ref()
            .is_none_or(|modal| modal.submitting)
        {
            return;
        }
        self.set_start_chat_mode(mode, cx);
        if let Some(modal) = &self.start_chat_modal {
            modal.picker_focus(cx).focus(window, cx);
        }
    }

    pub(crate) fn new_terminal_tab(
        &mut self,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_cwd = scope.project_id().and_then(|project_id| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .map(|project| project.cwd.as_str())
        });
        let cwd = terminal_working_dir(
            None,
            project_cwd,
            &self.settings(cx).default_working_dir,
            runner_backend::app_paths::home_dir()
                .as_deref()
                .and_then(|home| home.to_str()),
        );
        let mut spawned_id = None;
        let result = (|| -> Result<String> {
            let spawned = runner_backend::ops::session::session_start_shell_in(
                self.core(cx),
                scope,
                cwd,
                Some(INITIAL_COLS),
                Some(INITIAL_ROWS),
            )?;
            spawned_id = Some(spawned.id.clone());
            self.refresh_sessions(cx);
            self.reload_tabs(cx)?;
            if !self.tabs.activate_session(&spawned.id) {
                bail!("terminal tab was not created");
            }
            self.sync_active_project_from_active_tab(cx);
            self.set_route(AppRoute::Chat, cx);
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.chat_error = None;
                self.mark_active_tab_viewed(window, cx);
                self.sync_active_chat_detail(cx);
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
            }
            Err(error) => {
                if let Some(session_id) = spawned_id {
                    let _ = runner_backend::ops::session::session_close(self.core(cx), &session_id);
                }
                self.refresh_sessions(cx);
                let _ = self.reload_tabs(cx);
                self.chat_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mission_drawer_available = matches!(self.route, AppRoute::Mission(_))
            && self.mission_workspace.read(cx).drawer_available(cx);
        let target = new_terminal_target(
            &self.route,
            mission_drawer_available,
            self.active_tab_is_terminal(cx),
        );
        if target == NewTerminalTarget::MissionDrawer {
            self.mission_workspace
                .update(cx, |workspace, workspace_cx| {
                    workspace.add_terminal_drawer_shell(window, workspace_cx)
                });
            return;
        }
        let Some(original) = self.tabs.active().cloned() else {
            self.chat_error = Some("Open a tab before creating a terminal".into());
            cx.notify();
            return;
        };
        if self.route != AppRoute::Chat {
            self.set_route(AppRoute::Chat, cx);
            self.activate_tab(&original.id, window, cx);
        }
        if target == NewTerminalTarget::ChatDrawer {
            self.add_terminal_drawer_shell(window, cx);
            return;
        }
        if let Some(pane_id) = new_terminal_empty_pane(&original) {
            let (scope, cwd) = self.terminal_start_location(cx);
            self.spawn_terminal_in_pane(pane_id, original, scope, cwd, window, cx);
        } else {
            self.split_pane(&original.focused_pane_id, SplitOrientation::Row, window, cx);
        }
    }

    pub(crate) fn toggle_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.route, AppRoute::Mission(_)) {
            self.mission_workspace
                .update(cx, |workspace, workspace_cx| {
                    workspace.toggle_terminal_drawer(window, workspace_cx)
                });
            return;
        }
        if self.route != AppRoute::Chat || self.active_tab_is_terminal(cx) {
            return;
        }
        let Some(layout) = self.tabs.active() else {
            return;
        };
        if layout.drawer_open() {
            self.hide_terminal_drawer(window, cx);
        } else if layout.drawer_shells().is_empty() {
            self.add_terminal_drawer_shell(window, cx);
        } else {
            if let Some(layout) = self.tabs.active_mut() {
                layout.set_drawer_open(true);
            }
            self.finish_terminal_drawer_update(window, cx);
        }
    }

    pub(crate) fn hide_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(layout) = self.tabs.active_mut() {
            layout.set_drawer_open(false);
        }
        self.finish_terminal_drawer_update(window, cx);
    }

    pub(crate) fn activate_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.activate_drawer_shell(session_id))
        {
            return;
        }
        match self.persist_active_tab(cx) {
            Ok(()) => {
                self.chat_error = None;
                if let Some(layout) = self.tabs.active().cloned() {
                    if let Err(error) =
                        self.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                    {
                        self.chat_error = Some(error.to_string());
                    }
                }
                self.focus_drawer_terminal(session_id, window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn add_terminal_drawer_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(original) = self.tabs.active().cloned() else {
            self.chat_error = Some("Open a chat tab before creating a terminal".into());
            cx.notify();
            return;
        };
        if self.active_tab_is_terminal(cx) {
            return;
        }
        let (scope, cwd) = self.terminal_start_location(cx);
        let size = self.estimated_drawer_terminal_size(&original, window, cx);
        let mut spawned_id = None;
        let result = (|| -> Result<String> {
            let spawned = runner_backend::ops::session::session_start_shell_in(
                self.core(cx),
                scope,
                cwd,
                Some(size.0),
                Some(size.1),
            )?;
            spawned_id = Some(spawned.id.clone());
            self.refresh_sessions(cx);
            self.tabs
                .active_mut()
                .context("active tab disappeared")?
                .add_drawer_shell(spawned.id.clone());
            self.persist_active_tab(cx)?;
            self.reload_tabs(cx)?;
            self.set_route(AppRoute::Chat, cx);
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.chat_error = None;
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
                self.focus_drawer_terminal(&session_id, window, cx);
            }
            Err(error) => {
                if let Some(session_id) = spawned_id {
                    let _ = runner_backend::ops::session::session_close(self.core(cx), &session_id);
                }
                if let Ok(input) = original.upsert_input() {
                    let _ = runner_backend::ops::node::node_tab_upsert(self.core(cx), input);
                }
                self.refresh_sessions(cx);
                let _ = self.reload_tabs(cx);
                self.chat_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn finish_terminal_drawer_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self
            .persist_active_tab(cx)
            .and_then(|_| self.reload_tabs(cx))
            .and_then(|_| self.ensure_active_tab_attached(window, cx));
        match result {
            Ok(()) => {
                self.chat_error = None;
                if self.tabs.active().is_some_and(PaneLayout::drawer_open) {
                    if let Some(session_id) = self
                        .tabs
                        .active()
                        .and_then(PaneLayout::active_drawer_shell)
                        .map(str::to_owned)
                    {
                        self.focus_drawer_terminal(&session_id, window, cx);
                    }
                } else {
                    self.focus_active_terminal(window, cx);
                }
            }
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// Where a terminal started into the active tab opens: the tab's
    /// project, and the focused sibling's cwd before the project's.
    pub(crate) fn terminal_start_location(&self, cx: &App) -> (ProjectScope, Option<String>) {
        let sibling_cwd = self.tabs.active().and_then(|layout| {
            layout
                .focused_session_id()
                .or_else(|| {
                    layout
                        .root
                        .leaves()
                        .into_iter()
                        .find_map(|leaf| leaf.session_id.as_deref())
                })
                .and_then(|session_id| self.session_start_cwd(session_id, cx))
        });
        let project_id = self.active_tab_project_id(cx);
        let project_cwd = project_id.as_deref().and_then(|project_id| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .map(|project| project.cwd.as_str())
        });
        let cwd = terminal_working_dir(
            sibling_cwd.as_deref(),
            project_cwd,
            &self.settings(cx).default_working_dir,
            runner_backend::app_paths::home_dir()
                .as_deref()
                .and_then(|home| home.to_str()),
        );
        (ProjectScope::or_root(project_id), cwd)
    }

    pub(crate) fn spawn_terminal_in_pane(
        &mut self,
        pane_id: String,
        original: PaneLayout,
        scope: ProjectScope,
        cwd: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial_size = self
            .tabs
            .active()
            .map(|layout| self.estimated_terminal_size(layout, &pane_id, window, cx))
            .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
        let mut spawned_id = None;
        let result = (|| -> Result<String> {
            let spawned = runner_backend::ops::session::session_start_shell_in(
                self.core(cx),
                scope,
                cwd,
                Some(initial_size.0),
                Some(initial_size.1),
            )?;
            spawned_id = Some(spawned.id.clone());
            self.refresh_sessions(cx);
            self.tabs.assign_to_active(&pane_id, &spawned.id)?;
            self.persist_active_tab(cx)?;
            self.reload_tabs(cx)?;
            self.tabs.activate_session(&spawned.id);
            self.sync_active_project_from_active_tab(cx);
            self.set_route(AppRoute::Chat, cx);
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.chat_error = None;
                self.mark_active_tab_viewed(window, cx);
                self.sync_active_chat_detail(cx);
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
            }
            Err(error) => {
                if let Some(session_id) = spawned_id {
                    let _ = runner_backend::ops::session::session_close(self.core(cx), &session_id);
                }
                if let Ok(input) = original.upsert_input() {
                    let _ = runner_backend::ops::node::node_tab_upsert(self.core(cx), input);
                }
                self.refresh_sessions(cx);
                let _ = self.reload_tabs(cx);
                self.chat_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn open_new_tab_modal(
        &mut self,
        _: &NewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        let focused_empty_pane = self.focused_empty_chat_pane();
        if let Some(pane_id) = focused_empty_pane {
            self.open_pane_chat_modal(&pane_id, window, cx);
            return;
        }
        let active_project_id = self.active_project_id(cx);
        let project = active_project_id
            .as_deref()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(ChatTarget::NewTab, None, project, window, cx);
    }

    pub(crate) fn open_sidebar_chat_modal(
        &mut self,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        let project = scope
            .project_id()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(ChatTarget::NewTab, None, project, window, cx);
    }

    pub(crate) fn open_pane_chat_modal(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.tabs.active_tab_id().map(str::to_owned) else {
            return;
        };
        let tab_project_id = self.active_tab_project_id(cx);
        let project = tab_project_id
            .as_deref()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(
            ChatTarget::Pane {
                tab_id,
                pane_id: pane_id.to_owned(),
            },
            self.last_focused_role_id.clone(),
            project,
            window,
            cx,
        );
    }

    fn open_start_chat_modal(
        &mut self,
        target: ChatTarget,
        default_role_id: Option<String>,
        project: Option<runner_backend::repo::project::ProjectRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut error = None;
        match runner_backend::ops::role::role_list(self.core(cx)) {
            Ok(roles) => self
                .app_store
                .update(cx, |store, store_cx| store.replace_roles(roles, store_cx)),
            Err(load_error) => error = Some(load_error.to_string()),
        }

        let (runtimes, agents_checking, agents_error) =
            load_selectable_runtimes(self.core(cx), self.settings(cx));
        let persisted_mode = read_start_chat_mode(&self.core(cx).app_data_dir);
        let mode = if default_role_id.is_some() {
            ChatMode::Role
        } else {
            persisted_mode
        };
        let role_id = default_role_id
            .filter(|role_id| {
                self.app_store
                    .read(cx)
                    .roles
                    .iter()
                    .any(|role| role.id == *role_id)
            })
            .or_else(|| {
                self.app_store
                    .read(cx)
                    .roles
                    .first()
                    .map(|role| role.id.clone())
            });
        let runtime_name = runtimes
            .iter()
            .find(|runtime| runtime.name.key() == self.settings(cx).default_runtime)
            .or_else(|| runtimes.first())
            .map(|runtime| runtime.name.to_string());
        let title = match mode {
            ChatMode::Role => role_id
                .as_deref()
                .and_then(|role_id| {
                    self.app_store
                        .read(cx)
                        .roles
                        .iter()
                        .find(|role| role.id == role_id)
                })
                .map(|role| default_title_for_role(&role.handle))
                .unwrap_or_default(),
            ChatMode::Runtime => runtime_name
                .as_deref()
                .and_then(|name| runtimes.iter().find(|runtime| runtime.name.key() == name))
                .map(|runtime| default_title_for_runtime(&runtime.display_name))
                .unwrap_or_default(),
        };
        let cwd_placeholder = cwd_placeholder(
            mode,
            role_id.as_deref().and_then(|role_id| {
                self.app_store
                    .read(cx)
                    .roles
                    .iter()
                    .find(|role| role.id == role_id)
            }),
            &self.settings(cx).default_working_dir,
        );
        let title_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", title, false).text_size(theme::text_body())
        });
        let (scope, project_cwd) = project_start_scope(project.as_ref());
        let cwd_input = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), project_cwd, cwd_placeholder)
                .text_size(theme::text_ui())
        });
        let model_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", false).placeholder_as_value(true)
        });
        let root = cx.entity();
        let role_handler = selection_handler(&root, StartChatSelection::Role);
        let roles = self.app_store.read(cx).roles.clone();
        let selected_role = role_id
            .as_deref()
            .and_then(|id| roles.iter().find(|role| role.id == id));
        let role_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-role",
                select_cx.focus_handle(),
                role_id.clone().unwrap_or_default(),
                role_options(&roles),
                role_handler,
                select_cx,
            )
            .picker(true)
            .placeholder("No roles yet")
            .disabled(roles.is_empty())
        });
        let role_runtime_options = role_runtime_options(&runtimes, selected_role);
        let role_runtime_handler = selection_handler(&root, StartChatSelection::RoleRuntime);
        let role_runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-role-runtime",
                select_cx.focus_handle(),
                selected_role
                    .map(|role| role.runtime.clone())
                    .unwrap_or_default(),
                role_runtime_options,
                role_runtime_handler,
                select_cx,
            )
            .full_width(true)
            .min_menu_width(px(200.))
        });
        let runtime_handler = selection_handler(&root, StartChatSelection::Runtime);
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-runtime",
                select_cx.focus_handle(),
                runtime_name.clone().unwrap_or_default(),
                runtime_options(&runtimes),
                runtime_handler,
                select_cx,
            )
            .picker(true)
            .disabled(runtimes.is_empty())
        });
        let effort_handler = selection_handler(&root, StartChatSelection::Effort);
        let effort_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-effort",
                select_cx.focus_handle(),
                "",
                Vec::new(),
                effort_handler,
                select_cx,
            )
            .full_width(true)
            .min_menu_width(px(200.))
        });
        let speed_handler = selection_handler(&root, StartChatSelection::Speed);
        let speed_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-speed",
                select_cx.focus_handle(),
                "inherit",
                speed_options(None, false),
                speed_handler,
                select_cx,
            )
            .full_width(true)
            .min_menu_width(px(160.))
        });
        let model_field = cx.new(|model_cx| ModelField::new(model_input.clone(), &[], model_cx));
        let scroll_handle = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll_handle.clone(), scroll_owner));
        let role_mode_focus = cx.focus_handle();
        let direct_mode_focus = cx.focus_handle();
        let reset_focus = std::array::from_fn(|_| cx.focus_handle());
        let browse_focus = cx.focus_handle();
        let close_focus = cx.focus_handle();
        let cancel_focus = cx.focus_handle();
        let submit_focus = cx.focus_handle();
        // The model's dot, note and reset live outside the field, so the form
        // redraws with it.
        let model_subscription = cx.observe(&model_input, |this, _, cx| {
            if let Some(modal) = this.start_chat_modal.as_mut() {
                sync_model_marker(modal, cx);
                sync_effort_control(modal, cx);
            }
            cx.notify();
        });

        self.sidebar_preview_open = false;
        self.start_chat_modal = Some(StartChatModal {
            target,
            scope,
            mode,
            roles: self.app_store.read(cx).roles.clone(),
            runtimes,
            role_id,
            runtime_name,
            role_runtime_override: None,
            effort: String::new(),
            speed: "inherit".into(),
            title: title_input,
            cwd: cwd_input,
            model: model_input,
            model_field,
            role_select,
            role_runtime_select,
            runtime_select,
            effort_select,
            speed_select,
            scroll_handle,
            scrollbar,
            role_mode_focus,
            direct_mode_focus,
            reset_focus,
            browse_focus,
            close_focus,
            cancel_focus,
            submit_focus,
            agents_checking,
            agents_error,
            submitting: false,
            error: error.take(),
            _model_subscription: model_subscription,
        });
        if let Some(modal) = self.start_chat_modal.as_mut() {
            restore_baseline(modal, cx);
        }
        self.start_chat_modal
            .as_ref()
            .unwrap()
            .picker_focus(cx)
            .focus(window, cx);
        self.refresh_start_chat_models(cx);
        cx.notify();
    }

    fn refresh_start_chat_models(&self, cx: &Context<Self>) {
        if let Some(runtime) = self
            .start_chat_modal
            .as_ref()
            .and_then(|modal| modal.active_runtime())
            .filter(|runtime| self.settings(cx).model_runtimes().contains(&runtime.name))
        {
            runner_backend::ops::runtime::runtime_refresh_models(self.core(cx), &[runtime.name]);
        }
    }

    pub(crate) fn refresh_start_chat_runtimes(&mut self, cx: &mut Context<Self>) {
        let (runtimes, agents_checking, agents_error) =
            load_selectable_runtimes(self.core(cx), self.settings(cx));
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        let catalog_loaded = agents_error.is_none();
        let previous_runtime = modal.runtime_name.clone();
        let previous_override = modal.role_runtime_override.clone();
        modal.agents_checking = agents_checking;
        modal.agents_error = agents_error;

        if catalog_loaded {
            // A control the user has not changed follows the refreshed defaults.
            let effort_untouched = modal.effort == modal.baseline_effort();
            modal.runtimes = runtimes;
            if modal.role_runtime_override.as_ref().is_some_and(|name| {
                !modal
                    .runtimes
                    .iter()
                    .any(|runtime| runtime.name.key() == name.as_str())
            }) {
                modal.role_runtime_override = None;
            }
            if modal.runtime_name.as_ref().is_none_or(|name| {
                !modal
                    .runtimes
                    .iter()
                    .any(|runtime| runtime.name.key() == name.as_str())
            }) {
                modal.runtime_name = modal
                    .runtimes
                    .first()
                    .map(|runtime| runtime.name.to_string());
            }
            if modal.mode == ChatMode::Runtime && modal.runtime_name != previous_runtime {
                let derived = modal
                    .selected_runtime()
                    .map(|runtime| default_title_for_runtime(&runtime.display_name))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
            }
            modal.runtime_select.update(cx, |select, select_cx| {
                select.set_options(runtime_options(&modal.runtimes), select_cx);
                select.set_value(modal.runtime_name.clone().unwrap_or_default(), select_cx);
            });
            sync_role_runtime_select(modal, cx);
            if modal.runtime_name != previous_runtime
                || modal.role_runtime_override != previous_override
            {
                restore_baseline(modal, cx);
            } else {
                if effort_untouched {
                    modal.effort = modal.baseline_effort();
                }
                sync_runtime_controls(modal, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn sync_start_chat_default_runtime(&mut self, cx: &mut Context<Self>) {
        let default_runtime = self.settings(cx).default_runtime.clone();
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        let runtime_name = modal
            .runtimes
            .iter()
            .find(|runtime| runtime.name.key() == default_runtime)
            .or_else(|| modal.runtimes.first())
            .map(|runtime| runtime.name.to_string());
        if modal.runtime_name == runtime_name {
            return;
        }
        modal.runtime_name = runtime_name;
        if modal.mode == ChatMode::Runtime {
            let derived = modal
                .selected_runtime()
                .map(|runtime| default_title_for_runtime(&runtime.display_name))
                .unwrap_or_default();
            update_auto_title(&modal.title, derived, cx);
        }
        modal.runtime_select.update(cx, |select, select_cx| {
            select.set_value(modal.runtime_name.clone().unwrap_or_default(), select_cx)
        });
        restore_baseline(modal, cx);
        cx.notify();
    }

    pub(crate) fn remember_active_role(&mut self, cx: &App) {
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        self.last_focused_role_id = self
            .session_entry(&session_id, cx)
            .and_then(|entry| entry.role_id.clone());
    }

    fn close_start_chat_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .start_chat_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.start_chat_modal = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    fn set_start_chat_mode(&mut self, mode: ChatMode, cx: &mut Context<Self>) {
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let app_data_dir = self.core(cx).app_data_dir.clone();
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.mode == mode || modal.submitting {
            return;
        }
        modal.mode = mode;
        clear_role_overrides(modal, cx);
        let derived = match mode {
            ChatMode::Role => modal
                .selected_role()
                .map(|role| default_title_for_role(&role.handle))
                .unwrap_or_default(),
            ChatMode::Runtime => modal
                .selected_runtime()
                .map(|runtime| default_title_for_runtime(&runtime.display_name))
                .unwrap_or_default(),
        };
        update_auto_title(&modal.title, derived, cx);
        let placeholder = cwd_placeholder(mode, modal.selected_role(), &default_working_dir);
        modal.cwd.update(cx, |input, input_cx| {
            input.set_placeholder(placeholder, input_cx)
        });
        let _ = write_start_chat_mode(&app_data_dir, mode);
        self.refresh_start_chat_models(cx);
        cx.notify();
    }

    fn select_start_chat_choice(
        &mut self,
        selection: StartChatSelection,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let refresh_models = matches!(
            selection,
            StartChatSelection::Role
                | StartChatSelection::RoleRuntime
                | StartChatSelection::Runtime
        );
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        match selection {
            StartChatSelection::Role => {
                modal.role_id = Some(value.to_owned());
                let derived = modal
                    .selected_role()
                    .map(|role| default_title_for_role(&role.handle))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
                let placeholder =
                    cwd_placeholder(modal.mode, modal.selected_role(), &default_working_dir);
                modal.cwd.update(cx, |input, input_cx| {
                    input.set_placeholder(placeholder, input_cx)
                });
                // Overrides are on top of one role's setup: another role
                // starts from its own.
                clear_role_overrides(modal, cx);
            }
            StartChatSelection::RoleRuntime => {
                let own = modal
                    .selected_role()
                    .is_some_and(|role| role.runtime == value);
                let next = (!own && !value.is_empty()).then(|| value.to_owned());
                // This runs inside the Runtime select's own choose, which holds
                // the select: it already shows the pick, so it is not synced.
                if next != modal.role_runtime_override {
                    modal.role_runtime_override = next;
                    restore_baseline(modal, cx);
                }
            }
            StartChatSelection::Runtime => {
                modal.runtime_name = Some(value.to_owned());
                let derived = modal
                    .selected_runtime()
                    .map(|runtime| default_title_for_runtime(&runtime.display_name))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
                restore_baseline(modal, cx);
            }
            StartChatSelection::Effort => modal.effort = value.to_owned(),
            StartChatSelection::Speed => modal.speed = value.to_owned(),
        }
        if refresh_models {
            self.refresh_start_chat_models(cx);
        }
        cx.notify();
    }

    fn browse_start_chat_cwd(&mut self, cx: &mut Context<Self>) {
        if self
            .start_chat_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        let Some(cwd_input) = self
            .start_chat_modal
            .as_ref()
            .map(|modal| modal.cwd.clone())
        else {
            return;
        };
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Pick a working directory".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this.start_chat_modal.as_mut() else {
                    return;
                };
                if modal.cwd != cwd_input {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            modal.cwd.update(cx, |input, input_cx| {
                                input.reset(path.to_string_lossy().into_owned(), input_cx)
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_start_chat_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.close_start_chat_modal(window, cx);
            }
            "enter"
                if self
                    .start_chat_modal
                    .as_ref()
                    .is_some_and(|modal| !modal.is_composing(cx)) =>
            {
                cx.stop_propagation();
                self.submit_start_chat(window, cx);
            }
            _ => {}
        }
    }

    fn submit_start_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_ref() else {
            return;
        };
        if !modal.can_submit() || modal.is_composing(cx) {
            return;
        }
        let target = modal.target.clone();
        let cwd = effective_working_dir(
            modal.cwd.read(cx).text(),
            modal.mode == ChatMode::Role
                && modal
                    .selected_role()
                    .and_then(|role| role.working_dir.as_deref())
                    .is_some_and(|path| !path.trim().is_empty()),
            &self.settings(cx).default_working_dir,
        );
        // A control still showing its starting value is untouched.
        let model = modal.model_override(cx);
        let effort = modal.effort_override();
        let speed = modal.speed_override();
        let title = user_chat_title(modal.title.read(cx));
        let scope = modal.scope.clone();
        let request = build_start_request(
            modal.mode,
            modal
                .selected_role()
                .map(|role| (role.id.as_str(), role.runtime.as_str())),
            modal.selected_runtime().map(|runtime| runtime.name.key()),
            modal.role_runtime_override.as_deref(),
            model,
            effort,
            speed,
            cwd,
        )
        .expect("validated start chat selection");
        let initial_size = match &target {
            ChatTarget::NewTab => (INITIAL_COLS, INITIAL_ROWS),
            ChatTarget::Pane { tab_id, pane_id }
                if self.tabs.active_tab_id() == Some(tab_id.as_str()) =>
            {
                self.tabs
                    .active()
                    .map(|layout| self.estimated_terminal_size(layout, pane_id, window, cx))
                    .unwrap_or((INITIAL_COLS, INITIAL_ROWS))
            }
            ChatTarget::Pane { .. } => {
                if let Some(modal) = self.start_chat_modal.as_mut() {
                    modal.error = Some("The target tab is no longer active".into());
                }
                cx.notify();
                return;
            }
        };
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.submitting = true;
            modal.error = None;
            set_start_chat_controls_disabled(modal, true, cx);
        }
        cx.notify();

        let mut spawned_id = None;
        let mut rename_error = None;
        let result = (|| -> Result<String> {
            let spawned = match request {
                StartRequest::Role {
                    role_id,
                    runtime,
                    model,
                    effort,
                    speed,
                    cwd,
                } => runner_backend::ops::session::session_start_direct_with_speed(
                    self.core(cx),
                    role_id,
                    runtime,
                    model,
                    effort,
                    speed,
                    scope,
                    cwd,
                    Some(initial_size.0),
                    Some(initial_size.1),
                )?,
                StartRequest::Runtime {
                    runtime,
                    model,
                    effort,
                    speed,
                    cwd,
                } => {
                    runner_backend::ops::session::session_start_runtime_with_speed(
                        self.core(cx),
                        &runtime,
                        scope,
                        cwd,
                        Some(initial_size.0),
                        Some(initial_size.1),
                        model,
                        effort,
                        speed,
                    )?
                    .session
                }
            };
            spawned_id = Some(spawned.id.clone());
            if let Some(title) = title {
                if let Err(error) = runner_backend::ops::session::session_rename(
                    self.core(cx),
                    &spawned.id,
                    Some(title),
                ) {
                    rename_error = Some(format!(
                        "Chat started, but its title could not be saved: {error}"
                    ));
                }
            }
            self.refresh_sessions(cx);
            match target {
                ChatTarget::NewTab => {
                    self.reload_tabs(cx)?;
                    self.tabs.activate_session(&spawned.id);
                    self.sync_active_project_from_active_tab(cx);
                }
                ChatTarget::Pane { pane_id, .. } => {
                    self.tabs.assign_to_active(&pane_id, &spawned.id)?;
                    self.persist_active_tab(cx)?;
                    self.reload_tabs(cx)?;
                    self.tabs.activate_session(&spawned.id);
                    self.sync_active_project_from_active_tab(cx);
                }
            }
            self.set_route(AppRoute::Chat, cx);
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.start_chat_modal = None;
                self.error = rename_error;
                self.remember_active_role(cx);
                self.mark_active_tab_viewed(window, cx);
                self.sync_active_chat_detail(cx);
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
            }
            Err(start_error) => {
                if let Some(session_id) = spawned_id {
                    self.start_chat_modal = None;
                    let _ = self.reload_tabs(cx);
                    self.tabs.activate_session(&session_id);
                    self.sync_active_project_from_active_tab(cx);
                    self.set_route(AppRoute::Chat, cx);
                    let _ = self.ensure_active_tab_attached(window, cx);
                    self.remember_active_role(cx);
                    self.mark_active_tab_viewed(window, cx);
                    self.sync_active_chat_detail(cx);
                    self.begin_chat_transition(
                        &session_id,
                        chat_lifecycle::TransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                    self.error = Some(start_error.to_string());
                } else if let Some(modal) = self.start_chat_modal.as_mut() {
                    modal.submitting = false;
                    modal.error = Some(start_error.to_string());
                    set_start_chat_controls_disabled(modal, false, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_start_chat_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let modal = self.start_chat_modal.as_ref().expect("modal is open");
        let mode = modal.mode;
        let submitting = modal.submitting;
        let can_submit = modal.can_submit();
        let form_width = OverlayWidth::Custom(MODAL_WIDTH).body_width(window);
        let layout = CardLayout::new(form_width);

        let role_fields = self.render_role_fields(modal, layout, cx);
        let direct_fields = self.render_direct_fields(modal, layout, cx);

        let root = cx.entity();
        let browse_root = root.clone();
        let content = div()
            .debug_selector(|| "START_CHAT_FORM".into())
            .flex()
            .flex_col()
            .when(cfg!(windows), |content| content.w(rems(form_width / 16.)))
            .gap_5()
            .on_key_down(cx.listener(Self::on_start_chat_key_down))
            .children(modal.error.as_ref().map(|error| {
                div()
                    .rounded(rems(4. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_size(theme::text_ui())
                    .text_color(theme::danger())
                    .child(SharedString::from(error.clone()))
            }))
            .child(
                div()
                    .debug_selector(|| "START_CHAT_MODES".into())
                    .flex()
                    .w_full()
                    .p(rems(2. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .child(self.render_mode_button(
                        "Direct",
                        ChatMode::Runtime,
                        mode,
                        submitting,
                        modal.direct_mode_focus.clone(),
                        cx,
                    ))
                    .child(self.render_mode_button(
                        "Role",
                        ChatMode::Role,
                        mode,
                        submitting,
                        modal.role_mode_focus.clone(),
                        cx,
                    )),
            )
            .child(match mode {
                ChatMode::Role => role_fields,
                ChatMode::Runtime => direct_fields,
            })
            .child(
                Field::new("start-chat-title-field", "Chat name", modal.title.clone())
                    .focus_target(modal.title.read(cx).focus_handle())
                    .emphasized(true)
                    .tag("optional"),
            )
            .child(
                Field::new(
                    "start-chat-working-dir-field",
                    "Working directory",
                    WorkingDirField::new(
                        modal.cwd.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| this.browse_start_chat_cwd(cx));
                        }),
                    )
                    .browse_focus(modal.browse_focus.clone()),
                )
                .focus_target(modal.cwd.read(cx).focus_handle())
                .emphasized(true)
                .subtitle(working_dir_hint(mode, modal.selected_role())),
            );

        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Start a chat"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("A one-on-one terminal with an agent, no mission required."),
                    ),
            )
            .child(
                IconButton::new("close-start-chat", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close start chat")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
                    }),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-start-chat", "Cancel")
                    .shortcut("esc")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-start-chat",
                    if submitting {
                        "Starting…"
                    } else {
                        "Start chat"
                    },
                )
                .shortcut(keymap::fixed_shortcut("cmd-enter"))
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_start_chat(window, cx));
                }),
            );
        let modal_close_root = cx.entity();
        let modal_element = Modal::new(
            title,
            content,
            Rc::new(move |window, cx| {
                modal_close_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
            }),
        )
        .key_context("StartChat")
        .width(OverlayWidth::Custom(MODAL_WIDTH))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            start_chat_focus_order(modal, cx)
        })
        .scrollbar(modal.scroll_handle.clone(), modal.scrollbar.clone())
        .footer(footer);
        div()
            .absolute()
            .inset_0()
            .on_action(cx.listener(Self::confirm_start_chat))
            .on_action(cx.listener(|this, _: &StartChatDirect, window, cx| {
                this.switch_start_chat_mode(ChatMode::Runtime, window, cx);
            }))
            .on_action(cx.listener(|this, _: &StartChatRole, window, cx| {
                this.switch_start_chat_mode(ChatMode::Role, window, cx);
            }))
            .child(modal_element)
            .into_any_element()
    }

    /// Puts one role control back to the role's value and hands focus to it,
    /// since its Reset goes with the override.
    fn reset_start_chat_control(
        &mut self,
        kind: ResetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.submitting {
            return;
        }
        let focus = match kind {
            ResetKind::Runtime => {
                modal.role_runtime_override = None;
                sync_role_runtime_select(modal, cx);
                restore_baseline(modal, cx);
                modal.role_runtime_select.read(cx).focus_handle()
            }
            ResetKind::Model => {
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
                modal.model.read(cx).focus_handle()
            }
            ResetKind::Effort => {
                modal.effort = modal.baseline_effort();
                sync_effort_control(modal, cx);
                modal.effort_select.read(cx).focus_handle()
            }
            ResetKind::Speed => {
                modal.speed = modal.baseline_speed();
                sync_speed_control(modal, cx);
                modal.speed_select.read(cx).focus_handle()
            }
        };
        focus.focus(window, cx);
        if kind == ResetKind::Runtime {
            if let Some(runtime) = self
                .start_chat_modal
                .as_ref()
                .and_then(|modal| modal.active_runtime())
            {
                self.request_model_catalog(runtime.name.key(), cx);
            }
        }
        cx.notify();
    }

    fn render_role_fields(
        &self,
        modal: &StartChatModal,
        layout: CardLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = card(layout).child(modal.role_select.clone());
        let role = modal.selected_role();
        if let Some(role) = role {
            card = card.child(self.render_role_setup(modal, role, layout, cx));
        }
        let mut field = Field::new(
            "start-chat-role-field",
            "Role",
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(card)
                .when(modal.roles.is_empty(), |field| {
                    field.child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::warning())
                            .child("No roles yet. Create one from the Roles page first."),
                    )
                }),
        )
        .focus_target(modal.role_select.read(cx).focus_handle())
        .emphasized(true);
        if role.is_some() {
            field = field.subtitle(ROLE_HINT);
        }
        field.into_any_element()
    }

    /// The role's Runtime, Model and Effort as controls filled with the role's
    /// values; an override adds the amber dot and, under the control, the
    /// role's value with Reset.
    fn render_role_setup(
        &self,
        modal: &StartChatModal,
        role: &Role,
        layout: CardLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let third = layout.third();
        let root = cx.entity();
        let overrides = modal.role_overrides(cx);
        let own = modal.on_own_agent();
        let agent = modal.role_agent().unwrap_or(&role.runtime).to_owned();
        let runtime = modal.active_runtime();
        let effort_available = runtime.is_some_and(|runtime| !runtime.efforts.is_empty());
        let note = |label: &'static str, kind: ResetKind, changed: bool, value: String| {
            changed.then(|| {
                override_note(
                    label,
                    third,
                    format!("role: {value}"),
                    reset_action(&root, &modal.reset_focus[kind as usize], kind),
                )
            })
        };
        let row = || div().flex().items_start().gap(rems(COLUMN_GAP / 16.));
        let controls = row()
            .child(setup_field(
                third,
                "Runtime",
                modal.role_runtime_select.clone(),
                note(
                    "Runtime",
                    ResetKind::Runtime,
                    overrides.runtime,
                    runtime_display_name(&modal.runtimes, &role.runtime),
                ),
            ))
            .child(setup_field(
                third,
                "Model",
                div().w_full().child(modal.model_field.clone()),
                note(
                    "Model",
                    ResetKind::Model,
                    overrides.model,
                    model_placeholder(Some(role)),
                ),
            ))
            .when(effort_available, |controls| {
                controls.child(setup_field(
                    third,
                    "Effort",
                    modal.effort_select.clone(),
                    note(
                        "Effort",
                        ResetKind::Effort,
                        overrides.effort,
                        role_setting_label(Some(&modal.baseline_effort())).0,
                    ),
                ))
            });
        card_section()
            .when(cfg!(test), |section| {
                section.debug_selector(|| "START_CHAT_SETUP".into())
            })
            .child(controls)
            .when(modal.codex_speed_visible(), |section| {
                section.child(row().child(render_speed_column(
                    modal,
                    third,
                    note(
                        "Speed",
                        ResetKind::Speed,
                        overrides.speed,
                        speed_label(role.codex_speed).into(),
                    ),
                )))
            })
            .children(speed_note(modal))
            .children((!own).then(|| {
                let note = agent_note(
                    &runtime_display_name(&modal.runtimes, &agent),
                    &runtime_display_name(&modal.runtimes, &role.runtime),
                    trimmed(role.model.as_deref()),
                    trimmed(role.effort.as_deref()),
                );
                div()
                    .w(rems(layout.content / 16.))
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(note)
            }))
            .into_any_element()
    }

    fn render_direct_fields(
        &self,
        modal: &StartChatModal,
        layout: CardLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings_root = cx.entity();
        let mut card = card(layout).child(modal.runtime_select.clone());
        if let Some(runtime) = modal.active_runtime() {
            let has_effort = !runtime.efforts.is_empty();
            let has_speed = modal.codex_speed_visible();
            card = card.child(
                card_section()
                    .when(cfg!(test), |section| {
                        section.debug_selector(|| "START_CHAT_SETUP".into())
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(rems(COLUMN_GAP / 16.))
                            .child(setup_field(
                                layout.model(has_effort, has_speed),
                                "Model",
                                div().w_full().child(modal.model_field.clone()),
                                None,
                            ))
                            .when(has_effort, |row| {
                                row.child(setup_field(
                                    EFFORT_COLUMN_WIDTH,
                                    "Effort",
                                    modal.effort_select.clone(),
                                    None,
                                ))
                            })
                            .when(has_speed, |row| {
                                row.child(render_speed_column(modal, SPEED_COLUMN_WIDTH, None))
                            }),
                    )
                    .children(speed_note(modal)),
            );
        }
        Field::new(
            "start-chat-direct-agent-field",
            "Agent",
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(card)
                .when(modal.runtimes.is_empty(), |field| {
                    field.child(
                        div()
                            .flex()
                            .items_center()
                            .text_size(theme::text_meta())
                            .text_color(theme::warning())
                            .when(modal.agents_checking, |message| {
                                message.child("Detecting agents…")
                            })
                            .when(!modal.agents_checking, |message| {
                                let root = settings_root.clone();
                                message.child("No enabled agents detected. ").child(
                                    div()
                                        .id("start-chat-open-agent-settings")
                                        .cursor_pointer()
                                        .text_color(theme::warning())
                                        .hover(|link| link.text_color(theme::text()))
                                        .child("Configure one in Settings → Agents.")
                                        .on_click(move |_, window, cx| {
                                            root.update(cx, |this, cx| {
                                                this.start_chat_modal = None;
                                                this.enter_settings_route(
                                                    Some("agents"),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }),
                                )
                            }),
                    )
                })
                .children(modal.agents_error.clone().map(|error| {
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::danger())
                        .child(error)
                })),
        )
        .focus_target(modal.runtime_select.read(cx).focus_handle())
        .emphasized(true)
        .into_any_element()
    }

    fn render_mode_button(
        &self,
        label: &'static str,
        mode: ChatMode,
        active: ChatMode,
        disabled: bool,
        focus_handle: FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let key_root = root.clone();
        let mut button = div()
            .debug_selector(move || match mode {
                ChatMode::Role => "START_CHAT_ROLE_MODE".into(),
                ChatMode::Runtime => "START_CHAT_DIRECT_MODE".into(),
            })
            .id(match mode {
                ChatMode::Role => "start-chat-mode-role",
                ChatMode::Runtime => "start-chat-mode-runtime",
            })
            .track_focus(&focus_handle.tab_stop(!disabled && active == mode))
            .tab_index(0)
            .tab_stop(!disabled && active == mode)
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .h(rems(30. / 16.))
            .rounded_md()
            .text_size(theme::text_ui())
            .font_weight(FontWeight::SEMIBOLD)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .text_color(if active == mode {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active == mode, |button| button.bg(theme::border()))
            .opacity(if disabled { 0.6 } else { 1. })
            .focus_visible(|button| button.text_color(theme::text()));
        if !disabled {
            button = button
                .cursor_pointer()
                .hover(|button| button.text_color(theme::text()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.mouse_switch_start_chat_mode(mode, window, cx);
                }))
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "left" | "right") {
                        cx.stop_propagation();
                        let next = if mode == ChatMode::Role {
                            ChatMode::Runtime
                        } else {
                            ChatMode::Role
                        };
                        key_root.update(cx, |this, cx| {
                            this.set_start_chat_mode(next, cx);
                            if let Some(modal) = &this.start_chat_modal {
                                modal.mode_focus().focus(window, cx);
                            }
                        });
                    } else if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        key_root.update(cx, |this, cx| this.set_start_chat_mode(mode, cx));
                    }
                });
        }
        button
            .gap_2()
            .child(label)
            .child(
                div()
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::faint())
                    .child(keymap::fixed_shortcut(match mode {
                        ChatMode::Runtime => "cmd-1",
                        ChatMode::Role => "cmd-2",
                    })),
            )
            .into_any_element()
    }
}

impl NativeRoot {
    /// The cwd a shell started from `session_id` opens in: see
    /// `shell_start_cwd`. Only shell sessions scan for OSC 7, so a chat's
    /// cwd never moves.
    pub(crate) fn session_start_cwd(&self, session_id: &str, cx: &App) -> Option<String> {
        let live = self
            .app_store
            .read(cx)
            .bridge
            .session(session_id)
            .and_then(|terminal| terminal.live_cwd());
        shell_start_cwd(
            live,
            self.session_entry(session_id, cx)
                .and_then(|entry| entry.cwd.as_deref()),
        )
    }
}

/// Where a new shell opens when it is started from another session (#575):
/// the directory that session's shell last reported through OSC 7, which
/// `live_cwd` only offers while it exists, else the cwd it was spawned in.
/// `None` continues down `terminal_working_dir`'s chain.
pub(crate) fn shell_start_cwd(live: Option<PathBuf>, spawn: Option<&str>) -> Option<String> {
    live.and_then(|cwd| cwd.into_os_string().into_string().ok())
        .or_else(|| spawn.map(str::to_owned))
}

pub(crate) fn terminal_working_dir(
    sibling_cwd: Option<&str>,
    project_cwd: Option<&str>,
    default_cwd: &str,
    home: Option<&str>,
) -> Option<String> {
    [sibling_cwd, project_cwd, Some(default_cwd), home]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|path| !path.is_empty())
        .map(str::to_owned)
}

fn selection_handler(root: &Entity<NativeRoot>, selection: StartChatSelection) -> SelectHandler {
    let root = root.clone();
    Rc::new(move |value, _, cx| {
        root.update(cx, |this, cx| {
            this.select_start_chat_choice(selection, &value, cx)
        });
    })
}

fn role_options(roles: &[Role]) -> Vec<SelectOption> {
    roles
        .iter()
        .map(|role| {
            SelectOption::new(role.id.clone(), role.display_name.clone())
                .description(format!("@{}", role.handle))
                .leading(role_leading(&role.handle))
        })
        .collect()
}

fn role_leading(handle: &str) -> SelectLeading {
    let seed = handle.to_owned();
    SelectLeading::new(format!("role:{handle}"), move |size| {
        RoleAvatar::new(seed.clone(), size).into_any_element()
    })
}

fn runtime_leading(runtime: &str) -> SelectLeading {
    let name = runtime.to_owned();
    SelectLeading::new(format!("runtime:{runtime}"), move |size| {
        runtime_mark(&name, size)
    })
}

/// The agents a picker offers: the name, the command in monospace under it,
/// and the provider's mark.
fn runtime_options(runtimes: &[RuntimeCatalogEntry]) -> Vec<SelectOption> {
    runtimes
        .iter()
        .map(|runtime| {
            SelectOption::new(runtime.name.to_string(), runtime.display_name.clone())
                .description(runtime.command.clone())
                .leading(runtime_leading(runtime.name.key()))
        })
        .collect()
}

/// The agents a role chat can run on: every selectable one, plus the role's
/// own when it is not. The role's own is the untouched choice.
fn role_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    role: Option<&Role>,
) -> Vec<SelectOption> {
    let mut options = runtime_options(runtimes)
        .into_iter()
        .map(|option| SelectOption {
            description: None,
            marked: role.is_some_and(|role| role.runtime != option.value),
            ..option
        })
        .collect::<Vec<_>>();
    if let Some(role) =
        role.filter(|role| options.iter().all(|option| option.value != role.runtime))
    {
        options.push(
            SelectOption::new(
                role.runtime.clone(),
                runtime_display_name(runtimes, &role.runtime),
            )
            .leading(runtime_leading(&role.runtime)),
        );
    }
    options
}

fn option_select_options(options: &[RuntimeCatalogOption]) -> Vec<SelectOption> {
    options
        .iter()
        .map(|option| {
            let mut select = SelectOption::new(option.value.clone(), option.label.clone());
            if let Some(description) = &option.description {
                select = select.description(description.clone());
            }
            select
        })
        .collect()
}

fn parse_speed(value: &str) -> Option<CodexSpeed> {
    match value {
        "standard" => Some(CodexSpeed::Standard),
        "fast" => Some(CodexSpeed::Fast),
        _ => None,
    }
}

fn speed_label(speed: Option<CodexSpeed>) -> &'static str {
    match speed {
        None => "Inherit",
        Some(CodexSpeed::Standard) => "Standard",
        Some(CodexSpeed::Fast) => "Fast",
    }
}

/// Speed's choices. A role that sets a speed has no Inherit: it could not undo
/// the role's value. On the role's own Codex agent a choice that differs from
/// `baseline` carries the amber dot.
fn speed_options(baseline: Option<&str>, role_sets_speed: bool) -> Vec<SelectOption> {
    let option = |value: &'static str, label: &'static str| {
        SelectOption::new(value, label).marked(baseline.is_some_and(|baseline| baseline != value))
    };
    let mut options = Vec::new();
    if !role_sets_speed {
        options.push(option("inherit", "Inherit"));
    }
    options.push(option("standard", "Standard"));
    options.push(option("fast", "Fast"));
    options
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// What a blank Model reads: the role's explicit model while the chat runs on
/// the role's own agent, else `default` because no model reaches the launch.
fn model_placeholder(role: Option<&Role>) -> String {
    role.and_then(|role| trimmed(role.model.as_deref()))
        .unwrap_or("default")
        .to_owned()
}

/// The role whose model, effort and speed the controls start from: only the
/// role the chat runs on the agent of.
fn inheriting_role(modal: &StartChatModal) -> Option<&Role> {
    (modal.mode == ChatMode::Role && modal.on_own_agent())
        .then(|| modal.selected_role())
        .flatten()
}

/// The model that will reach the launch: the one typed, else the role's own,
/// which the backend keeps when only an effort is overridden. The `default`
/// placeholder is not one: nothing is sent for it.
fn launch_model(modal: &StartChatModal, cx: &App) -> Option<String> {
    normalized_value(modal.model.read(cx).text()).or_else(|| {
        inheriting_role(modal)
            .and_then(|role| trimmed(role.model.as_deref()))
            .map(str::to_owned)
    })
}

/// Whether the agent drops an effort that has no model beside it. The launch
/// gives Antigravity `--effort` only together with `--model`, and only for a
/// pair its catalog lists, so its effort waits for a model that reaches the
/// launch.
fn effort_needs_launch_model(runtime: &RuntimeCatalogEntry) -> bool {
    runtime.capabilities.effort_needs_launch_model
}

fn sync_role_runtime_select(modal: &StartChatModal, cx: &mut Context<NativeRoot>) {
    let options = role_runtime_options(&modal.runtimes, modal.selected_role());
    let value = modal.role_agent().unwrap_or_default().to_owned();
    modal.role_runtime_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(value, select_cx);
    });
}

/// Back to the role's own setup: its agent, model, effort and speed.
fn clear_role_overrides(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    modal.role_runtime_override = None;
    sync_role_runtime_select(modal, cx);
    restore_baseline(modal, cx);
}

/// Every control back to what it starts from: no model typed, and the Effort
/// and Speed that leave them untouched.
fn restore_baseline(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    modal
        .model
        .update(cx, |input, input_cx| input.reset("", input_cx));
    modal.effort = modal.baseline_effort();
    modal.speed = modal.baseline_speed();
    sync_runtime_controls(modal, cx);
}

fn sync_runtime_controls(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let runtime = modal.active_runtime().cloned();
    let models = runtime
        .as_ref()
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default();
    modal.model_field.update(cx, |field, field_cx| {
        field.set_suggestions(models, field_cx);
        field.set_disabled(modal.submitting || runtime.is_none(), field_cx);
    });
    let placeholder = model_placeholder(inheriting_role(modal));
    modal.model.update(cx, |input, input_cx| {
        input.set_placeholder(placeholder, input_cx)
    });
    sync_model_marker(modal, cx);
    sync_effort_control(modal, cx);
    sync_speed_control(modal, cx);
}

/// The model carries the override dot while a role chat on the role's own
/// agent has a model other than the role's.
fn sync_model_marker(modal: &StartChatModal, cx: &mut Context<NativeRoot>) {
    let marked = modal.role_overrides(cx).model;
    modal
        .model_field
        .update(cx, |field, field_cx| field.set_marker(marked, field_cx));
}

fn sync_speed_control(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let visible = modal.codex_speed_visible();
    if !visible {
        modal.speed = "inherit".into();
    }
    let baseline = modal.baseline_speed();
    let role = inheriting_role(modal).filter(|role| {
        crate::runtime_ui::catalog_capabilities(&modal.runtimes, &role.runtime).codex_speed
    });
    let options = speed_options(
        role.map(|_| baseline.as_str()),
        role.is_some_and(|role| role.codex_speed.is_some()),
    );
    if !options.iter().any(|option| option.value == modal.speed) {
        modal.speed = baseline;
    }
    modal.speed_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(modal.speed.clone(), select_cx);
        select.set_disabled(modal.submitting || !visible, select_cx);
    });
}

/// Effort's choices: the agent's levels for the model the chat will run (its
/// default model when none is set, every level when that is unknown too), the
/// role's own level even when the catalog does not list it, and a blank
/// `default` only while no level is known to start from. An agent that drops
/// an effort without a model offers none until one reaches the launch. On the
/// role's own agent a choice other than the role's carries the amber dot.
fn sync_effort_control(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let baseline = modal.baseline_effort();
    let inherits = inheriting_role(modal).is_some();
    let launch = launch_model(modal, cx);
    let levels = modal
        .active_runtime()
        .map(|runtime| {
            let needs_model = effort_needs_launch_model(runtime);
            let default_model = (!needs_model)
                .then(|| trimmed(runtime.default_model.as_deref()))
                .flatten();
            match launch.as_deref().or(default_model) {
                Some(model) => runtime.efforts_for_model(model),
                None if needs_model => Vec::new(),
                None => runtime.efforts.clone(),
            }
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|level| !level.value.is_empty())
        .collect::<Vec<_>>();
    let mut options = option_select_options(&levels);
    if baseline.is_empty() {
        options.insert(0, SelectOption::new("", "default"));
    } else if !levels.iter().any(|level| level.value == baseline) {
        options.insert(0, SelectOption::new(baseline.clone(), baseline.clone()));
    }
    for option in &mut options {
        option.marked = inherits && option.value != baseline;
    }
    if !options.iter().any(|option| option.value == modal.effort) {
        modal.effort = baseline;
    }
    let disabled = modal.submitting || options.len() <= 1;
    modal.effort_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(modal.effort.clone(), select_cx);
        select.set_disabled(disabled, select_cx);
    });
}

fn set_start_chat_controls_disabled(
    modal: &mut StartChatModal,
    disabled: bool,
    cx: &mut Context<NativeRoot>,
) {
    modal
        .title
        .update(cx, |field, field_cx| field.set_disabled(disabled, field_cx));
    modal
        .cwd
        .update(cx, |field, field_cx| field.set_disabled(disabled, field_cx));
    modal.role_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled || modal.roles.is_empty(), select_cx)
    });
    modal.role_runtime_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled, select_cx)
    });
    modal.runtime_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled || modal.runtimes.is_empty(), select_cx)
    });
    sync_runtime_controls(modal, cx);
}

fn start_chat_focus_order(modal: &StartChatModal, cx: &App) -> Vec<FocusHandle> {
    let mut order = vec![modal.close_focus.clone(), modal.mode_focus()];
    let controls = |order: &mut Vec<FocusHandle>| {
        if let Some(runtime) = modal.active_runtime() {
            order.push(modal.model.read(cx).focus_handle());
            if !runtime.efforts.is_empty() {
                order.push(modal.effort_select.read(cx).focus_handle());
            }
        }
        if modal.codex_speed_visible() {
            order.push(modal.speed_select.read(cx).focus_handle());
        }
    };
    match modal.mode {
        ChatMode::Role => {
            if !modal.roles.is_empty() {
                order.push(modal.role_select.read(cx).focus_handle());
            }
            if modal.selected_role().is_some() {
                order.push(modal.role_runtime_select.read(cx).focus_handle());
                controls(&mut order);
                let overrides = modal.role_overrides(cx);
                for (shown, focus) in [
                    (
                        overrides.runtime,
                        &modal.reset_focus[ResetKind::Runtime as usize],
                    ),
                    (
                        overrides.model,
                        &modal.reset_focus[ResetKind::Model as usize],
                    ),
                    (
                        overrides.effort,
                        &modal.reset_focus[ResetKind::Effort as usize],
                    ),
                    (
                        overrides.speed,
                        &modal.reset_focus[ResetKind::Speed as usize],
                    ),
                ] {
                    if shown {
                        order.push(focus.clone());
                    }
                }
            }
        }
        ChatMode::Runtime => {
            if !modal.runtimes.is_empty() {
                order.push(modal.runtime_select.read(cx).focus_handle());
            }
            controls(&mut order);
        }
    }
    order.push(modal.title.read(cx).focus_handle());
    order.push(modal.cwd.read(cx).focus_handle());
    order.push(modal.browse_focus.clone());
    order.push(modal.cancel_focus.clone());
    if modal.can_submit() {
        order.push(modal.submit_focus.clone());
    }
    order
}

/// Column widths inside a card, from the width the modal leaves its form.
#[derive(Clone, Copy)]
struct CardLayout {
    /// The card's own width, border included.
    outer: f32,
    /// The width inside the border and the card's padding.
    content: f32,
}

impl CardLayout {
    fn new(form_width: f32) -> Self {
        Self {
            outer: form_width.floor(),
            content: form_width.floor() - 2. - 2. * CARD_PADDING,
        }
    }

    /// Runtime, Model and Effort of a role chat.
    fn third(self) -> f32 {
        ((self.content - 2. * COLUMN_GAP) / 3.).floor()
    }

    /// Direct's Model fills what its Effort and Speed leave.
    fn model(self, effort: bool, speed: bool) -> f32 {
        let taken = if effort {
            EFFORT_COLUMN_WIDTH + COLUMN_GAP
        } else {
            0.
        } + if speed {
            SPEED_COLUMN_WIDTH + COLUMN_GAP
        } else {
            0.
        };
        self.content - taken
    }
}

/// A role or an agent: who the chat is with on top, how it runs underneath.
fn card(layout: CardLayout) -> Div {
    div()
        .when(cfg!(test), |card| {
            card.debug_selector(|| "START_CHAT_CARD".into())
        })
        .w(rems(layout.outer / 16.))
        .flex()
        .flex_col()
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
}

fn card_section() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rems(COLUMN_GAP / 16.))
        .px(rems(CARD_PADDING / 16.))
        .py(rems(CARD_PADDING / 16.))
        .border_t_1()
        .border_color(theme::border())
}

/// A labelled column of a card, with a note under the control while it
/// overrides the role's value.
fn setup_field(
    width: f32,
    label: &'static str,
    control: impl IntoElement,
    note: Option<AnyElement>,
) -> Div {
    div()
        .when(cfg!(test), |field| {
            field.debug_selector(move || format!("START_CHAT_FIELD {label}"))
        })
        .w(rems(width / 16.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .child(section_label(label))
        .child(control)
        .children(note)
}

/// What Reset takes of a column's width, and the gap before it.
const RESET_WIDTH: f32 = 50.;
const RESET_GAP: f32 = 8.;

/// The role's value under an overridden control, with its Reset.
fn override_note(
    label: &'static str,
    width: f32,
    text: String,
    reset: gpui::Stateful<Div>,
) -> AnyElement {
    let selector = format!("START_CHAT_NOTE {label} {text}");
    div()
        .flex()
        .items_center()
        .gap(rems(RESET_GAP / 16.))
        .child(
            column_text(text, width - RESET_WIDTH - RESET_GAP)
                .when(cfg!(test), |note| {
                    note.debug_selector(move || selector.clone())
                })
                .text_size(theme::text_meta())
                .text_color(theme::faint()),
        )
        .child(reset)
        .into_any_element()
}

/// A control's Reset: restores that one value to the role's.
fn reset_action(
    root: &Entity<NativeRoot>,
    focus: &FocusHandle,
    kind: ResetKind,
) -> gpui::Stateful<Div> {
    let root = root.clone();
    text_action(
        match kind {
            ResetKind::Runtime => "start-chat-reset-runtime",
            ResetKind::Model => "start-chat-reset-model",
            ResetKind::Effort => "start-chat-reset-effort",
            ResetKind::Speed => "start-chat-reset-speed",
        },
        focus,
        move |window, cx| {
            root.update(cx, |this, cx| {
                this.reset_start_chat_control(kind, window, cx)
            });
        },
    )
    .when(cfg!(test), move |reset| {
        reset.debug_selector(move || format!("START_CHAT_RESET {kind:?}"))
    })
    .flex_none()
    .gap(rems(6. / 16.))
    .text_size(theme::text_ui())
    .text_color(theme::muted())
    .hover(|reset| reset.text_color(theme::text()))
    .child(
        svg()
            .flex_none()
            .path("rotate-ccw.svg")
            .size(rems(11. / 16.))
            .text_color(theme::muted()),
    )
    .child("Reset")
}

fn render_speed_column(modal: &StartChatModal, width: f32, note: Option<AnyElement>) -> Div {
    setup_field(
        width,
        "Speed",
        div()
            .when(cfg!(test), |field| {
                field.debug_selector(|| "START_CHAT_SPEED_FIELD".into())
            })
            .child(modal.speed_select.clone()),
        note,
    )
}

fn speed_note(modal: &StartChatModal) -> Option<Div> {
    (modal.effective_speed() == Some(CodexSpeed::Fast)).then(|| {
        div()
            .when(cfg!(test), |note| {
                note.debug_selector(|| "START_CHAT_SPEED_NOTE".into())
            })
            .text_size(theme::text_meta())
            .text_color(theme::faint())
            .child("Fast uses more credits.")
    })
}

/// Why a role's model and effort are not on show once another agent is
/// picked.
fn agent_note(
    agent: &str,
    role_agent: &str,
    role_model: Option<&str>,
    role_effort: Option<&str>,
) -> String {
    let start = format!("{agent} starts from its own model and effort.");
    match (role_model, role_effort) {
        (Some(model), Some(effort)) => format!(
            "{start} The role's {model} and {effort} belong to {role_agent} and don't carry over."
        ),
        (Some(value), None) | (None, Some(value)) => {
            format!("{start} The role's {value} belongs to {role_agent} and doesn't carry over.")
        }
        (None, None) => start,
    }
}

/// Where a blank Working directory starts.
fn working_dir_hint(mode: ChatMode, role: Option<&Role>) -> &'static str {
    match mode {
        ChatMode::Role
            if role.is_some_and(|role| trimmed(role.working_dir.as_deref()).is_some()) =>
        {
            "Blank starts in the role's directory."
        }
        _ => "Blank starts in your default directory.",
    }
}

pub(crate) fn load_selectable_runtimes(
    core: &AppCore,
    settings: &AppSettings,
) -> (Vec<RuntimeCatalogEntry>, bool, Option<String>) {
    let checking = core
        .runtime_discovery
        .read()
        .map(|discovery| discovery.checking)
        .unwrap_or(false);
    match runner_backend::ops::runtime::runtime_catalog(core) {
        Ok(catalog) => {
            let enabled = catalog
                .iter()
                .filter(|runtime| settings.is_agent_enabled(runtime.name, runtime.default_enabled))
                .map(|runtime| runtime.name.to_string())
                .collect::<Vec<_>>();
            (
                filter_selectable_runtime_catalog(catalog, Some(&enabled)),
                checking,
                None,
            )
        }
        Err(error) => (Vec::new(), checking, Some(error.to_string())),
    }
}

fn runtime_display_name(runtimes: &[RuntimeCatalogEntry], name: &str) -> String {
    runtimes
        .iter()
        .find(|runtime| runtime.name.key() == name)
        .map(|runtime| runtime.display_name.clone())
        .or_else(|| {
            runner_backend::ops::runtime::runtime_list()
                .into_iter()
                .find(|runtime| runtime.name.key() == name)
                .map(|runtime| runtime.display_name)
        })
        .unwrap_or_else(|| name.to_owned())
}

fn default_title_for_role(handle: &str) -> String {
    format!("@{handle}")
}

fn default_title_for_runtime(label: &str) -> String {
    label.to_owned()
}

fn user_chat_title(input: &TextField) -> Option<String> {
    input
        .edited()
        .then(|| normalized_value(input.text()))
        .flatten()
}

fn update_auto_title(title: &Entity<TextField>, derived: String, cx: &mut gpui::App) {
    title.update(cx, |input, input_cx| {
        input.set_placeholder(derived, input_cx);
    });
}

fn cwd_placeholder(mode: ChatMode, role: Option<&Role>, default_path: &str) -> String {
    match mode {
        ChatMode::Role => working_dir_placeholder(
            role.and_then(|role| role.working_dir.as_deref()),
            default_path,
        ),
        ChatMode::Runtime => working_dir_placeholder(None, default_path),
    }
}

fn project_start_scope(
    project: Option<&runner_backend::repo::project::ProjectRow>,
) -> (ProjectScope, String) {
    project.map_or_else(
        || (ProjectScope::Root, String::new()),
        |project| {
            (
                ProjectScope::Project(project.id.clone()),
                project.cwd.clone(),
            )
        },
    )
}

#[cfg(test)]
fn effort_options_for_runtime<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtimes
        .iter()
        .find(|runtime| runtime.name.key() == name)
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default()
}

fn normalized_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The launch request. A role chat with nothing overridden sends no override;
/// a model or effort changed on the role's own agent sends that agent with the
/// changed values, which the backend resolves by keeping the role's others;
/// another agent goes with its own defaults.
#[allow(clippy::too_many_arguments)]
fn build_start_request(
    mode: ChatMode,
    role: Option<(&str, &str)>,
    runtime_name: Option<&str>,
    role_runtime_override: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
    speed: Option<CodexSpeed>,
    cwd: Option<String>,
) -> Option<StartRequest> {
    match mode {
        ChatMode::Role => role.map(|(role_id, role_runtime)| StartRequest::Role {
            role_id: role_id.to_owned(),
            runtime: role_runtime_override
                .or((model.is_some() || effort.is_some()).then_some(role_runtime))
                .map(str::to_owned),
            model,
            effort,
            speed,
            cwd,
        }),
        ChatMode::Runtime => runtime_name.map(|runtime| StartRequest::Runtime {
            runtime: runtime.to_owned(),
            model,
            effort,
            speed,
            cwd,
        }),
    }
}

fn start_chat_mode_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(START_CHAT_MODE_FILE)
}

fn read_start_chat_mode(app_data_dir: &Path) -> ChatMode {
    let value = fs::read_to_string(start_chat_mode_path(app_data_dir)).ok();
    ChatMode::from_persisted(value.as_deref())
}

fn write_start_chat_mode(app_data_dir: &Path, mode: ChatMode) -> std::io::Result<()> {
    fs::write(start_chat_mode_path(app_data_dir), mode.persisted())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    struct ModalHost(Entity<NativeRoot>);
    impl Render for ModalHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |root, cx| {
                if root.route == AppRoute::Settings {
                    return root.render_app_shell(window, cx);
                }
                let pane = root
                    .tabs
                    .active()
                    .cloned()
                    .filter(|layout| {
                        layout
                            .root
                            .leaves()
                            .iter()
                            .any(|leaf| leaf.session_id.is_some())
                    })
                    .map(|layout| root.render_pane_node(&layout.root, &layout, window, cx));
                div()
                    .size_full()
                    .children(pane)
                    .key_context("Terminal")
                    .track_focus(&root.root_focus)
                    .on_action(cx.listener(NativeRoot::open_new_tab_modal))
                    .on_action(cx.listener(NativeRoot::new_terminal_action))
                    .on_action(cx.listener(NativeRoot::new_mission_action))
                    .on_action(cx.listener(|root, _: &SelectTab1, _, cx| {
                        root.tabs.activate("01M3VD00000000000000000001");
                        cx.notify();
                    }))
                    .on_action(cx.listener(|root, _: &SelectTab2, _, cx| {
                        root.tabs.activate("01M3VD00000000000000000002");
                        cx.notify();
                    }))
                    .when(root.start_chat_modal.is_some(), |host| {
                        host.child(root.render_start_chat_modal(window, cx))
                    })
                    .when(root.start_mission_modal.is_some(), |host| {
                        host.child(root.render_start_mission_modal(cx))
                    })
                    .into_any_element()
            })
        }
    }

    pub(in crate::surfaces) struct ModalHarness {
        pub(in crate::surfaces) visual: VisualTestContext,
        host: WindowHandle<ModalHost>,
        _cx: TestAppContext,
        _temp: tempfile::TempDir,
        _theme: crate::theme_snapshot::ThemeGuard,
    }

    /// A modal window that opens already holding `roles` and `runtimes`, so no
    /// frame is drawn from the machine's own catalog: a test frame keeps every
    /// debug selector it ever drew.
    pub(in crate::surfaces) fn modal_harness(
        width: f32,
        height: f32,
        roles: Vec<Role>,
        runtimes: Vec<RuntimeCatalogEntry>,
        mode: ChatMode,
    ) -> ModalHarness {
        let theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
            app_data_dir: temp.path().to_owned(),
            sessions: session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            usage: Arc::new(runner_backend::usage::UsageService::default()),
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        };
        let mut cx = TestAppContext::single();
        cx.update(|cx| keymap::install_bindings(cx, &keymap::KeymapOverrides::new(), false));
        #[cfg(not(windows))]
        let updater = cx.new(|cx| Updater::new(false, cx));
        #[cfg(windows)]
        let updater = cx.new(|cx| Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(GlobalUpdater(updater));
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                temp.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });
        cx.set_global(GlobalAppStore(store.clone()));
        cx.set_global(WindowLayoutCheckpoint::default());
        let host = cx.add_window(|window, cx| {
            let root = cx.new(|cx| {
                NativeRoot::new(
                    "test-chat-modal".into(),
                    temp.path().to_owned(),
                    Some("chats".into()),
                    None,
                    store,
                    window,
                    cx,
                )
            });
            root.update(cx, |root, cx| {
                root.root_focus.focus(window, cx);
                root.open_new_tab_modal(&NewTab, window, cx);
                seed_modal(
                    root.start_chat_modal.as_mut().unwrap(),
                    roles,
                    runtimes,
                    mode,
                    cx,
                );
                root.start_chat_modal
                    .as_ref()
                    .unwrap()
                    .picker_focus(cx)
                    .focus(window, cx);
            });
            ModalHost(root)
        });
        cx.run_until_parked();
        let visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(width), px(height)));
        visual.run_until_parked();
        ModalHarness {
            visual,
            host,
            _cx: cx,
            _temp: temp,
            _theme: theme,
        }
    }

    /// Puts roles and agents in the modal as a loaded catalog would.
    fn seed_modal(
        modal: &mut StartChatModal,
        roles: Vec<Role>,
        runtimes: Vec<RuntimeCatalogEntry>,
        mode: ChatMode,
        cx: &mut Context<NativeRoot>,
    ) {
        modal.roles = roles;
        modal.runtimes = runtimes;
        modal.role_id = modal.roles.first().map(|role| role.id.clone());
        modal.runtime_name = modal
            .runtimes
            .first()
            .map(|runtime| runtime.name.to_string());
        modal.mode = mode;
        modal.role_select.update(cx, |select, select_cx| {
            select.set_options(role_options(&modal.roles), select_cx);
            select.set_value(modal.role_id.clone().unwrap_or_default(), select_cx);
            select.set_disabled(modal.roles.is_empty(), select_cx);
        });
        modal.runtime_select.update(cx, |select, select_cx| {
            select.set_options(runtime_options(&modal.runtimes), select_cx);
            select.set_value(modal.runtime_name.clone().unwrap_or_default(), select_cx);
            select.set_disabled(modal.runtimes.is_empty(), select_cx);
        });
        sync_role_runtime_select(modal, cx);
        restore_baseline(modal, cx);
    }

    impl ModalHarness {
        pub(in crate::surfaces) fn act(
            &mut self,
            f: impl FnOnce(&mut NativeRoot, &mut Window, &mut Context<NativeRoot>),
        ) {
            self.host
                .update(&mut self.visual, |host, window, cx| {
                    host.0.update(cx, |root, cx| f(root, window, cx));
                    cx.notify();
                })
                .unwrap();
            self.visual.run_until_parked();
        }

        fn edit(&mut self, f: impl FnOnce(&mut StartChatModal, &mut Context<NativeRoot>)) {
            self.act(|root, _, cx| {
                f(root.start_chat_modal.as_mut().unwrap(), cx);
                cx.notify();
            });
        }

        fn read<R>(&self, f: impl FnOnce(&StartChatModal, &App) -> R) -> R {
            self.host
                .read_with(&self.visual, |host, cx| {
                    f(host.0.read(cx).start_chat_modal.as_ref().unwrap(), cx)
                })
                .unwrap()
        }

        fn bounds(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
            self.visual
                .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        }

        /// Opens the focused select with the keyboard and picks by key, the
        /// way a person does: through `StyledSelect::choose`, which holds the
        /// select while it calls back.
        fn pick(&mut self, select: fn(&StartChatModal, &App) -> FocusHandle, keys: &str) {
            let handle = self.read(select);
            self.act(move |_, window, cx| handle.focus(window, cx));
            self.visual.simulate_keystrokes(keys);
            self.visual.run_until_parked();
        }

        /// Picks as the select would, minus the keyboard: it holds the choice
        /// itself before it calls back.
        fn choose(&mut self, selection: StartChatSelection, value: &str) {
            let value = value.to_owned();
            self.act(move |root, _, cx| {
                let modal = root.start_chat_modal.as_ref().unwrap();
                let select = match selection {
                    StartChatSelection::Role => &modal.role_select,
                    StartChatSelection::RoleRuntime => &modal.role_runtime_select,
                    StartChatSelection::Runtime => &modal.runtime_select,
                    StartChatSelection::Effort => &modal.effort_select,
                    StartChatSelection::Speed => &modal.speed_select,
                }
                .clone();
                select.update(cx, |select, cx| select.set_value(value.clone(), cx));
                root.select_start_chat_choice(selection, &value, cx);
            });
        }

        fn type_model(&mut self, text: &'static str) {
            self.edit(|modal, cx| modal.model.update(cx, |input, cx| input.set_text(text, cx)));
        }

        /// Clicks a control's Reset, as a person does.
        fn reset(&mut self, kind: ResetKind) {
            let bounds = self
                .bounds(&format!("START_CHAT_RESET {kind:?}"))
                .expect("the control shows its Reset");
            let content = self.bounds("MODAL_CONTENT").expect("modal content");
            assert!(content.contains(&bounds.origin) && content.contains(&bounds.bottom_right()));
            self.visual
                .simulate_click(bounds.center(), gpui::Modifiers::default());
            self.visual.run_until_parked();
        }

        /// The label the Effort control shows, and whether it carries the dot.
        fn effort_choice(&self) -> (String, bool) {
            self.read(|modal, cx| {
                let option = modal.effort_select.read(cx).selected().unwrap();
                (option.label.to_string(), option.marked)
            })
        }

        fn effort_disabled(&self) -> bool {
            self.read(|modal, cx| modal.effort_select.read(cx).is_disabled())
        }

        fn overrides(&self) -> RoleOverrides {
            self.read(|modal, cx| modal.role_overrides(cx))
        }
    }

    use gpui::{size, Render, TestAppContext, VisualTestContext, WindowHandle};
    use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
    use std::sync::{Mutex, RwLock};

    fn test_role(handle: &str, runtime: &str) -> Role {
        Role {
            id: format!("role-{handle}"),
            handle: handle.into(),
            display_name: format!("Role {handle}"),
            runtime: runtime.into(),
            command: runtime.into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// Claude Code and Codex as a detected catalog offers them.
    fn agents() -> Vec<RuntimeCatalogEntry> {
        let mut claude = runtime("claude-code", &["", "low", "high", "xhigh"]);
        claude.display_name = "Claude Code".into();
        claude.command = "claude".into();
        claude.default_effort = Some("high".into());
        let mut codex = runtime("codex", &["", "low", "medium"]);
        codex.display_name = "Codex".into();
        codex.default_model = Some("gpt-5.5".into());
        codex.default_effort = Some("medium".into());
        vec![claude, codex]
    }

    /// Antigravity as the catalog lists it: a default model, and levels per model.
    fn antigravity() -> RuntimeCatalogEntry {
        let mut agy = runtime("antigravity", &["", "low", "medium", "high"]);
        agy.display_name = "Antigravity CLI".into();
        agy.command = "agy".into();
        agy.default_model = Some("gemini-3.1-pro".into());
        agy.models = vec![RuntimeCatalogOption {
            value: "gemini-3.1-pro".into(),
            label: "Gemini 3.1 Pro".into(),
            description: None,
            supported_efforts: Some(vec!["low".into(), "high".into()]),
        }];
        agy
    }

    fn claude_role() -> Role {
        Role {
            model: Some("opus[1m]".into()),
            effort: Some("xhigh".into()),
            ..test_role("impl-claude", "claude-code")
        }
    }

    fn codex_role() -> Role {
        Role {
            codex_speed: Some(CodexSpeed::Fast),
            ..test_role("impl-codex", "codex")
        }
    }

    #[test]
    fn mode_switch_and_direct_speed_follow_the_selected_agent() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), agents(), ChatMode::Runtime);
        for mode in [ChatMode::Runtime, ChatMode::Role, ChatMode::Runtime] {
            modal.act(move |root, _, cx| root.set_start_chat_mode(mode, cx));
            let form = modal.bounds("START_CHAT_FORM").unwrap();
            let modes = modal.bounds("START_CHAT_MODES").unwrap();
            let role = modal.bounds("START_CHAT_ROLE_MODE").unwrap();
            let direct = modal.bounds("START_CHAT_DIRECT_MODE").unwrap();
            assert!(direct.origin.x < role.origin.x);
            assert!(modes.size.width >= px(480.), "{mode:?}: {modes:?}");
            assert_eq!(modes.size.width, form.size.width, "{mode:?}: {form:?}");
            assert!((role.size.width - direct.size.width).abs() <= px(1.));
            assert!(role.size.width > px(200.));
        }
        assert!(!modal.read(|modal, _| modal.codex_speed_visible()));

        modal.choose(StartChatSelection::Runtime, "codex");
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
        assert!(modal.bounds("START_CHAT_SPEED_NOTE").is_none());
        modal.choose(StartChatSelection::Speed, "fast");
        assert!(modal.bounds("START_CHAT_SPEED_NOTE").is_some());

        modal.choose(StartChatSelection::Runtime, "claude-code");
        assert_eq!(
            modal.read(|modal, _| (modal.codex_speed_visible(), modal.speed.clone())),
            (false, "inherit".into())
        );
    }

    #[test]
    fn a_role_chat_with_untouched_overrides_sends_none() {
        let role = Some(("coder", "claude-code"));
        assert_eq!(
            build_start_request(
                ChatMode::Role,
                role,
                Some("codex"),
                None,
                None,
                None,
                None,
                Some("/repo".into()),
            ),
            Some(StartRequest::Role {
                role_id: "coder".into(),
                runtime: None,
                model: None,
                effort: None,
                speed: None,
                cwd: Some("/repo".into()),
            })
        );
        // Speed alone does not pin the role's agent.
        assert_eq!(
            build_start_request(
                ChatMode::Role,
                role,
                None,
                None,
                None,
                None,
                Some(CodexSpeed::Fast),
                None
            ),
            Some(StartRequest::Role {
                role_id: "coder".into(),
                runtime: None,
                model: None,
                effort: None,
                speed: Some(CodexSpeed::Fast),
                cwd: None,
            })
        );
    }

    #[test]
    fn a_model_or_effort_changed_on_the_roles_agent_sends_that_agent_with_the_change() {
        let role = Some(("coder", "claude-code"));
        for (model, effort) in [
            (Some("sonnet"), None),
            (None, Some("low")),
            (Some("sonnet"), Some("low")),
        ] {
            assert_eq!(
                build_start_request(
                    ChatMode::Role,
                    role,
                    None,
                    None,
                    model.map(Into::into),
                    effort.map(Into::into),
                    None,
                    None,
                ),
                Some(StartRequest::Role {
                    role_id: "coder".into(),
                    runtime: Some("claude-code".into()),
                    model: model.map(Into::into),
                    effort: effort.map(Into::into),
                    speed: None,
                    cwd: None,
                })
            );
        }
    }

    #[test]
    fn another_agent_goes_with_its_own_defaults_and_direct_chats_start_as_before() {
        assert_eq!(
            build_start_request(
                ChatMode::Role,
                Some(("coder", "claude-code")),
                None,
                Some("codex"),
                None,
                None,
                None,
                None,
            ),
            Some(StartRequest::Role {
                role_id: "coder".into(),
                runtime: Some("codex".into()),
                model: None,
                effort: None,
                speed: None,
                cwd: None,
            })
        );
        assert_eq!(
            build_start_request(
                ChatMode::Runtime,
                Some(("coder", "claude-code")),
                Some("codex"),
                Some("claude-code"),
                Some("gpt-5.6-sol".into()),
                Some("high".into()),
                Some(CodexSpeed::Standard),
                None,
            ),
            Some(StartRequest::Runtime {
                runtime: "codex".into(),
                model: Some("gpt-5.6-sol".into()),
                effort: Some("high".into()),
                speed: Some(CodexSpeed::Standard),
                cwd: None,
            })
        );
        assert_eq!(
            build_start_request(
                ChatMode::Runtime,
                None,
                Some("codex"),
                None,
                None,
                None,
                None,
                None,
            ),
            Some(StartRequest::Runtime {
                runtime: "codex".into(),
                model: None,
                effort: None,
                speed: None,
                cwd: None,
            })
        );
        assert_eq!(
            build_start_request(ChatMode::Role, None, None, None, None, None, None, None),
            None
        );
    }

    #[test]
    fn an_untouched_role_chat_shows_the_roles_values_and_sends_no_override() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        assert!(modal.bounds("START_CHAT_SETUP").is_some());
        assert_eq!(modal.overrides(), RoleOverrides::default());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
            assert_eq!(modal.speed_override(), None);
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.effort, "xhigh");
            assert_eq!(model_placeholder(inheriting_role(modal)), "opus[1m]");
            assert!(modal.model.read(cx).placeholder_uses_value_style());
        });
        assert_eq!(modal.effort_choice(), ("xhigh".into(), false));

        // A control still showing the role's value is untouched, however it got there.
        modal.type_model("opus[1m]");
        modal.choose(StartChatSelection::Effort, "low");
        modal.choose(StartChatSelection::Effort, "xhigh");
        assert_eq!(modal.overrides(), RoleOverrides::default());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
        });
    }

    #[test]
    fn a_changed_value_carries_the_dot_and_its_reset_restores_only_that_value() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.type_model("sonnet");
        modal.choose(StartChatSelection::Effort, "low");
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                runtime: false,
                model: true,
                effort: true,
                speed: false
            }
        );
        assert!(modal.bounds("MODEL_FIELD_MARK").is_some());
        assert_eq!(modal.effort_choice(), ("low".into(), true));
        assert!(modal
            .bounds("START_CHAT_NOTE Model role: opus[1m]")
            .is_some());
        assert!(modal.bounds("START_CHAT_NOTE Effort role: xhigh").is_some());

        modal.reset(ResetKind::Model);
        modal.read(|modal, cx| {
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "low", "Reset restores one value only");
        });
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                effort: true,
                ..RoleOverrides::default()
            }
        );

        modal.reset(ResetKind::Effort);
        assert_eq!(modal.read(|modal, _| modal.effort.clone()), "xhigh");
        assert_eq!(modal.effort_choice(), ("xhigh".into(), false));
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn choosing_the_roles_own_agent_is_not_an_override() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        assert_eq!(
            modal.read(|modal, _| modal.role_runtime_override.clone()),
            Some("codex".into())
        );
        modal.choose(StartChatSelection::RoleRuntime, "claude-code");
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.role_runtime_select.read(cx).value(), "claude-code");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn another_role_starts_from_its_own_setup() {
        let other = Role {
            model: Some("gpt-5.5".into()),
            ..test_role("other", "codex")
        };
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![claude_role(), other.clone()],
            agents(),
            ChatMode::Role,
        );
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        modal.type_model("gpt-5.6-sol");
        modal.choose(StartChatSelection::Role, &other.id);
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "medium", "the codex agent's own default");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn another_agent_shows_its_own_defaults_at_full_contrast_and_only_runtime_carries_the_dot() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                runtime: true,
                ..RoleOverrides::default()
            }
        );
        assert!(modal.read(|modal, cx| {
            modal
                .role_runtime_select
                .read(cx)
                .selected()
                .unwrap()
                .marked
        }));
        assert!(modal
            .bounds("START_CHAT_NOTE Runtime role: Claude Code")
            .is_some());
        assert!(modal.bounds("START_CHAT_RESET Runtime").is_some());
        modal.read(|modal, cx| {
            assert!(inheriting_role(modal).is_none());
            // The agent's defaults, not the role's model and effort. The
            // model stays generic because no concrete value reaches launch.
            assert_eq!(model_placeholder(inheriting_role(modal)), "default");
            assert_eq!(modal.effort, "medium");
            assert_eq!(modal.model_override(cx), None);
            assert_eq!(modal.effort_override(), None);
        });
        assert_eq!(modal.effort_choice(), ("medium".into(), false));

        // Changing the agent's model is not the role's to override: no dot.
        modal.type_model("gpt-5.6-sol");
        assert!(modal.bounds("MODEL_FIELD_MARK").is_none());
        modal.read(|modal, cx| {
            assert_eq!(modal.model_override(cx), Some("gpt-5.6-sol".into()));
        });

        modal.reset(ResetKind::Runtime);
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.model.read(cx).text(), "");
            assert_eq!(modal.effort, "xhigh");
            assert_eq!(modal.role_runtime_select.read(cx).value(), "claude-code");
        });
        assert_eq!(modal.overrides(), RoleOverrides::default());
    }

    #[test]
    fn a_role_without_a_model_or_effort_starts_from_its_agents_defaults() {
        let modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "claude-code")],
            agents(),
            ChatMode::Role,
        );
        modal.read(|modal, cx| {
            assert_eq!(
                model_placeholder(inheriting_role(modal)),
                "default",
                "the role does not pin a model"
            );
            assert_eq!(modal.effort, "high", "the agent's default effort");
            assert_eq!(modal.effort_override(), None);
            assert_eq!(modal.model_override(cx), None);
            assert!(modal.model.read(cx).placeholder_uses_value_style());
        });
        assert_eq!(modal.effort_choice(), ("high".into(), false));
        drop(modal);

        // No default effort known: the control reads `default`, never a blank.
        let mut claude = agents().remove(0);
        claude.default_effort = None;
        let modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "claude-code")],
            vec![claude],
            ChatMode::Role,
        );
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        modal.read(|modal, _| assert_eq!(modal.effort, ""));
    }

    #[test]
    fn direct_model_placeholder_does_not_expose_the_cached_default() {
        let modal = modal_harness(
            1200.,
            1000.,
            Vec::new(),
            vec![agents().remove(1)],
            ChatMode::Runtime,
        );
        modal.read(|modal, cx| {
            assert_eq!(
                modal.active_runtime().unwrap().default_model.as_deref(),
                Some("gpt-5.5")
            );
            assert_eq!(model_placeholder(inheriting_role(modal)), "default");
            assert!(modal.model.read(cx).placeholder_uses_value_style());
        });
    }

    #[test]
    fn speed_options_mark_values_that_override_the_role() {
        let values = |options: Vec<SelectOption>| {
            options
                .iter()
                .map(|option| (option.value.clone(), option.marked))
                .collect::<Vec<_>>()
        };
        // A role that sets a speed cannot be told to inherit: no such choice.
        assert_eq!(
            values(speed_options(Some("fast"), true)),
            [("standard".to_owned(), true), ("fast".to_owned(), false)]
        );
        assert_eq!(
            values(speed_options(Some("inherit"), false)),
            [
                ("inherit".to_owned(), false),
                ("standard".to_owned(), true),
                ("fast".to_owned(), true)
            ]
        );
        assert_eq!(
            values(speed_options(None, false)),
            [
                ("inherit".to_owned(), false),
                ("standard".to_owned(), false),
                ("fast".to_owned(), false)
            ]
        );
    }

    #[test]
    fn effort_is_changeable_without_typing_a_model() {
        // Direct on an agent whose default model is not known.
        let mut modal = modal_harness(1200., 1000., Vec::new(), agents(), ChatMode::Runtime);
        assert_eq!(modal.effort_choice(), ("high".into(), false));
        modal.pick(
            |modal, cx| modal.effort_select.read(cx).focus_handle(),
            "enter down enter",
        );
        modal.read(|modal, _| {
            assert_eq!(modal.effort, "xhigh");
            assert_eq!(modal.effort_override(), Some("xhigh".into()));
        });
        drop(modal);

        // And on a role that has no model of its own.
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "claude-code")],
            agents(),
            ChatMode::Role,
        );
        modal.pick(
            |modal, cx| modal.effort_select.read(cx).focus_handle(),
            "enter up enter",
        );
        modal.read(|modal, _| {
            assert_eq!(modal.effort, "low");
            assert_eq!(modal.effort_override(), Some("low".into()));
        });
        assert!(modal.overrides().effort);
    }

    /// The launch gives Antigravity `--effort` only beside `--model`, so an
    /// effort with no model that reaches the launch would be shown and dropped.
    /// The `default` placeholder does not reach it.
    #[test]
    fn antigravity_effort_waits_for_a_model_that_reaches_the_launch() {
        let effort_picker: fn(&StartChatModal, &App) -> FocusHandle =
            |modal, cx| modal.effort_select.read(cx).focus_handle();

        // Direct: the runtime default is not pinned by the placeholder.
        let mut modal = modal_harness(
            1200.,
            1000.,
            Vec::new(),
            vec![antigravity()],
            ChatMode::Runtime,
        );
        modal.read(|modal, _| {
            assert_eq!(model_placeholder(inheriting_role(modal)), "default");
        });
        assert_eq!(modal.effort_choice(), ("default".into(), false));
        // No key is sent to it: Enter on a disabled select would submit the form.
        assert!(modal.effort_disabled());
        modal.read(|modal, _| {
            assert_eq!(modal.effort, "");
            assert_eq!(modal.effort_override(), None);
        });

        // A typed model is a pair the launch keeps.
        modal.type_model("gemini-3.1-pro");
        assert!(!modal.effort_disabled());
        modal.pick(effort_picker, "enter down enter");
        modal.read(|modal, cx| {
            assert_eq!(modal.effort_override(), Some("low".into()));
            assert_eq!(modal.model_override(cx), Some("gemini-3.1-pro".into()));
        });
        // Its levels are the model's own: no medium.
        modal.pick(effort_picker, "enter down enter");
        assert_eq!(modal.read(|modal, _| modal.effort.clone()), "high");

        // Clearing the model takes the effort with it.
        modal.type_model("");
        assert!(modal.effort_disabled());
        modal.read(|modal, _| {
            assert_eq!(modal.effort, "");
            assert_eq!(modal.effort_override(), None);
        });
        drop(modal);

        // A role with no model of its own has nothing for an effort to ride on.
        let modal = modal_harness(
            1200.,
            1000.,
            vec![test_role("plain", "antigravity")],
            vec![antigravity()],
            ChatMode::Role,
        );
        assert!(modal.effort_disabled());
        modal.read(|modal, _| assert_eq!(modal.effort_override(), None));
        assert!(!modal.overrides().effort);
        drop(modal);

        // A role's model reaches the launch, so its effort can change.
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![Role {
                model: Some("gemini-3.1-pro".into()),
                ..test_role("pro", "antigravity")
            }],
            vec![antigravity()],
            ChatMode::Role,
        );
        modal.pick(effort_picker, "enter down enter");
        modal.read(|modal, cx| {
            assert_eq!(modal.effort_override(), Some("low".into()));
            assert_eq!(modal.model_override(cx), None, "the role's model is kept");
        });
        drop(modal);

        // Another role sent to Antigravity has no model until one is typed.
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![claude_role()],
            [agents(), vec![antigravity()]].concat(),
            ChatMode::Role,
        );
        modal.choose(StartChatSelection::RoleRuntime, "antigravity");
        assert!(modal.effort_disabled());
        modal.read(|modal, _| assert_eq!(modal.effort_override(), None));
        modal.type_model("gemini-3.1-pro");
        assert!(!modal.effort_disabled());
        modal.pick(effort_picker, "enter down enter");
        modal.read(|modal, _| assert_eq!(modal.effort_override(), Some("low".into())));
    }

    #[test]
    fn role_speed_remains_visible_when_its_agent_is_not_selectable() {
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![codex_role()],
            vec![agents().remove(0)],
            ChatMode::Role,
        );
        assert!(modal.read(|modal, _| modal.codex_speed_visible()));
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
        modal.read(|modal, _| assert_eq!(modal.baseline_speed(), "fast"));
    }

    #[test]
    fn speed_shows_for_codex_roles_codex_overrides_and_direct_codex() {
        // A Codex role's speed is a control from the start.
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        assert!(modal.read(|modal, _| modal.codex_speed_visible()));
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
        assert!(modal.bounds("START_CHAT_SPEED_NOTE").is_some());
        modal.read(|modal, cx| {
            let option = modal.speed_select.read(cx).selected().unwrap();
            assert_eq!((option.label.as_ref(), option.marked), ("Fast", false));
            assert_eq!(modal.speed_override(), None);
        });
        modal.choose(StartChatSelection::Speed, "standard");
        assert_eq!(
            modal.overrides(),
            RoleOverrides {
                speed: true,
                ..RoleOverrides::default()
            }
        );
        assert!(modal.bounds("START_CHAT_NOTE Speed role: Fast").is_some());
        modal.reset(ResetKind::Speed);
        modal.read(|modal, _| {
            assert_eq!(modal.speed, "fast");
            assert_eq!(modal.speed_override(), None);
        });
        drop(modal);

        // A Claude role gains it by choosing Codex.
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        assert!(!modal.read(|modal, _| modal.codex_speed_visible()));
        modal.choose(StartChatSelection::RoleRuntime, "codex");
        assert!(modal.read(|modal, _| modal.codex_speed_visible()));
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
        modal.read(|modal, cx| {
            let option = modal.speed_select.read(cx).selected().unwrap();
            assert_eq!((option.label.as_ref(), option.marked), ("Inherit", false));
        });
        drop(modal);

        // Direct chats show it for Codex only.
        let mut modal = modal_harness(1200., 1000., Vec::new(), agents(), ChatMode::Runtime);
        assert!(!modal.read(|modal, _| modal.codex_speed_visible()));
        modal.choose(StartChatSelection::Runtime, "codex");
        assert!(modal.read(|modal, _| modal.codex_speed_visible()));
        assert!(modal.bounds("START_CHAT_SPEED_FIELD").is_some());
    }

    /// The pick goes through `StyledSelect::choose`, which holds the select
    /// while it calls back: a callback that touches that select again panics.
    #[test]
    fn every_select_takes_a_pick_made_from_its_open_menu() {
        let other = test_role("other", "codex");
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![claude_role(), other.clone()],
            agents(),
            ChatMode::Role,
        );
        let role_picker: fn(&StartChatModal, &App) -> FocusHandle =
            |modal, cx| modal.role_select.read(cx).focus_handle();
        modal.pick(role_picker, "enter down enter");
        assert_eq!(
            modal.read(|modal, _| modal.role_id.clone()),
            Some(other.id.clone())
        );
        modal.pick(role_picker, "enter up enter");
        assert_eq!(
            modal.read(|modal, _| modal.role_id.clone()),
            Some(claude_role().id)
        );

        let agent: fn(&StartChatModal, &App) -> FocusHandle =
            |modal, cx| modal.role_runtime_select.read(cx).focus_handle();
        // The role's own agent again is no override; another is.
        modal.pick(agent, "enter enter");
        assert_eq!(
            modal.read(|modal, _| modal.role_runtime_override.clone()),
            None
        );
        modal.pick(agent, "enter down enter");
        assert_eq!(
            modal.read(|modal, _| modal.role_runtime_override.clone()),
            Some("codex".into())
        );
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_select.read(cx).value(), "codex");
        });
        modal.pick(agent, "enter up enter");
        modal.read(|modal, cx| {
            assert_eq!(modal.role_runtime_override, None);
            assert_eq!(modal.role_runtime_select.read(cx).value(), "claude-code");
        });

        // xhigh is the role's; up two levels is low.
        modal.pick(
            |modal, cx| modal.effort_select.read(cx).focus_handle(),
            "enter up up enter",
        );
        assert_eq!(modal.read(|modal, _| modal.effort.clone()), "low");
        assert_eq!(modal.effort_choice(), ("low".into(), true));
        drop(modal);

        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        modal.pick(
            |modal, cx| modal.speed_select.read(cx).focus_handle(),
            "enter up enter",
        );
        assert_eq!(modal.read(|modal, _| modal.speed.clone()), "standard");
        drop(modal);

        let mut modal = modal_harness(1200., 1000., Vec::new(), agents(), ChatMode::Runtime);
        modal.pick(
            |modal, cx| modal.runtime_select.read(cx).focus_handle(),
            "enter down enter",
        );
        assert_eq!(
            modal.read(|modal, _| modal.runtime_name.clone()),
            Some("codex".into())
        );
    }

    /// The override dot has a slot of its own in a control, so a long value
    /// ends before it rather than running underneath.
    #[test]
    fn a_long_overridden_value_ends_before_the_dot() {
        let mut claude = agents().remove(0);
        claude.efforts.push(RuntimeCatalogOption {
            value: "an-effort-level-far-too-long-for-its-narrow-column".into(),
            label: "an-effort-level-far-too-long-for-its-narrow-column".into(),
            description: None,
            supported_efforts: None,
        });
        let mut modal = modal_harness(
            640.,
            480.,
            vec![claude_role()],
            vec![claude],
            ChatMode::Role,
        );
        modal.choose(
            StartChatSelection::Effort,
            "an-effort-level-far-too-long-for-its-narrow-column",
        );
        // Effort is the last select drawn, so its selectors are the ones held.
        let text = modal.bounds("STYLED_SELECT_TEXT").unwrap();
        let mark = modal.bounds("STYLED_SELECT_MARK").unwrap();
        let trigger = modal.bounds("STYLED_SELECT_TRIGGER").unwrap();
        assert!(text.right() <= mark.left(), "{text:?} {mark:?}");
        assert!(mark.right() <= trigger.right(), "{mark:?} {trigger:?}");

        // A long model reserves the same strip at the text field's right.
        modal.type_model("a-model-name-that-is-far-too-long-for-its-narrow-column-to-hold");
        let mark = modal.bounds("MODEL_FIELD_MARK").unwrap();
        let field = modal.bounds("START_CHAT_FIELD Model").unwrap();
        assert!(
            mark.left() >= field.left() && mark.right() <= field.right(),
            "{mark:?} in {field:?}"
        );
    }

    #[test]
    fn confirm_binding_precedes_every_control_and_preserves_open_menu_values() {
        for index in 0..15 {
            let mut modal = modal_harness(
                1200.,
                1000.,
                vec![codex_role(), claude_role()],
                agents(),
                ChatMode::Role,
            );
            modal.type_model("custom-model");
            modal.choose(StartChatSelection::Effort, "low");
            modal.choose(StartChatSelection::Speed, "standard");
            let focus = modal.read(|modal, cx| start_chat_focus_order(modal, cx)[index].clone());
            modal.act(|_, window, cx| focus.focus(window, cx));
            modal
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            modal.read(|modal, cx| {
                assert!(
                    modal.error.is_some(),
                    "confirm did not submit at control {index}"
                );
                assert_eq!(modal.model.read(cx).text(), "custom-model");
                assert_eq!(modal.effort, "low");
                assert_eq!(modal.speed, "standard");
            });
        }
        for picker in 0..5 {
            let mut modal = modal_harness(
                1200.,
                1000.,
                vec![codex_role(), claude_role()],
                agents(),
                ChatMode::Role,
            );
            if picker == 4 {
                let missing = modal
                    ._temp
                    .path()
                    .join("missing")
                    .to_string_lossy()
                    .into_owned();
                modal.edit(|form, cx| form.cwd.update(cx, |field, cx| field.set_text(missing, cx)));
                modal.act(|root, window, cx| {
                    root.switch_start_chat_mode(ChatMode::Runtime, window, cx)
                });
            }
            let select = modal.read(|modal, _| match picker {
                0 => modal.role_select.clone(),
                1 => modal.role_runtime_select.clone(),
                2 => modal.effort_select.clone(),
                3 => modal.speed_select.clone(),
                _ => modal.runtime_select.clone(),
            });
            let original = select.read_with(&modal.visual, |select, _| select.value().to_owned());
            let focus = select.read_with(&modal.visual, |select, _| select.focus_handle());
            modal.act(|_, window, cx| focus.focus(window, cx));
            modal.visual.simulate_keystrokes("enter down");
            modal
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            assert_eq!(
                select.read_with(&modal.visual, |select, _| select.value().to_owned()),
                original
            );
            modal.read(|modal, _| assert!(modal.error.is_some()));
        }
    }

    #[test]
    fn enter_opens_and_chooses_a_model_without_submitting_the_chat() {
        for mode in [ChatMode::Role, ChatMode::Runtime] {
            let mut catalog = agents();
            catalog
                .iter_mut()
                .find(|runtime| runtime.name == Runtime::Codex)
                .unwrap()
                .models = ["first-model", "second-model"]
                .into_iter()
                .map(|value| RuntimeCatalogOption {
                    value: value.into(),
                    label: value.into(),
                    description: None,
                    supported_efforts: None,
                })
                .collect();
            let mut modal = modal_harness(1200., 1000., vec![codex_role()], catalog, mode);
            if mode == ChatMode::Runtime {
                modal.choose(StartChatSelection::Runtime, "codex");
            }
            let missing = modal
                ._temp
                .path()
                .join("missing-model-key-test")
                .to_string_lossy()
                .into_owned();
            modal.edit(|form, cx| form.cwd.update(cx, |field, cx| field.set_text(missing, cx)));
            modal.pick(|form, cx| form.model.read(cx).focus_handle(), "enter");
            modal.read(|form, _| {
                assert!(form.error.is_none(), "Enter must open the model chooser");
                assert!(!form.submitting);
            });
            modal.visual.simulate_keystrokes("down enter");
            modal.read(|form, cx| {
                assert_eq!(form.model.read(cx).text(), "second-model");
                assert!(form.error.is_none(), "Choosing a model must not submit");
                assert!(!form.submitting);
            });
            modal
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            modal.read(|form, cx| {
                assert!(form.error.is_some(), "The confirm shortcut must submit");
                assert_eq!(form.model.read(cx).text(), "second-model");
            });
        }
    }

    #[test]
    fn confirm_from_model_menu_and_runtime_reset_preserves_the_shown_values() {
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        let mut catalog = agents();
        catalog[1].models = vec![RuntimeCatalogOption {
            value: "suggested".into(),
            label: "Suggested".into(),
            description: None,
            supported_efforts: None,
        }];
        modal.edit(|form, cx| seed_modal(form, vec![codex_role()], catalog, ChatMode::Role, cx));
        modal.type_model("custom");
        modal.pick(|form, cx| form.model.read(cx).focus_handle(), "down");
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        modal.read(|form, cx| {
            assert!(form.error.is_some());
            assert_eq!(form.model.read(cx).text(), "custom");
        });
        modal.choose(StartChatSelection::RoleRuntime, "claude-code");
        let focus = modal.read(|form, _| form.reset_focus[ResetKind::Runtime as usize].clone());
        modal.act(|_, window, cx| focus.focus(window, cx));
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        modal.read(|form, _| {
            assert!(form.error.is_some());
            assert_eq!(form.role_runtime_override.as_deref(), Some("claude-code"));
        });
    }

    #[test]
    fn remembered_and_preselected_roles_focus_the_picker_with_empty_modes_falling_back() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        modal.act(|root, window, cx| {
            let input =
                serde_json::from_value(serde_json::to_value(test_role("saved", "codex")).unwrap())
                    .unwrap();
            let role = runner_backend::ops::role::role_create(root.core(cx), input).unwrap();
            write_start_chat_mode(&root.core(cx).app_data_dir, ChatMode::Role).unwrap();
            root.close_start_chat_modal(window, cx);
            root.open_new_tab_modal(&NewTab, window, cx);
            let form = root.start_chat_modal.as_ref().unwrap();
            assert!(form.role_select.read(cx).focus_handle().is_focused(window));
            root.close_start_chat_modal(window, cx);
            write_start_chat_mode(&root.core(cx).app_data_dir, ChatMode::Runtime).unwrap();
            let mut layout = PaneLayout::single(None, &[]);
            layout.id = "01M3VD00000000000000000004".into();
            let pane_id = layout.focused_pane_id.clone();
            runner_backend::ops::node::node_tab_upsert(
                root.core(cx),
                layout.upsert_input().unwrap(),
            )
            .unwrap();
            root.reload_tabs(cx).unwrap();
            root.last_focused_role_id = Some(role.id);
            root.open_pane_chat_modal(&pane_id, window, cx);
            let form = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(form.mode, ChatMode::Role);
            assert!(form.role_select.read(cx).focus_handle().is_focused(window));
        });
        for mode in [ChatMode::Runtime, ChatMode::Role] {
            modal.edit(|form, cx| seed_modal(form, Vec::new(), Vec::new(), mode, cx));
            modal.act(|root, window, cx| {
                let form = root.start_chat_modal.as_ref().unwrap();
                form.picker_focus(cx).focus(window, cx);
                assert!(form.title.read(cx).focus_handle().is_focused(window));
            });
        }
        modal.edit(|form, cx| seed_modal(form, Vec::new(), agents(), ChatMode::Runtime, cx));
        modal.act(|root, window, cx| {
            let form = root.start_chat_modal.as_ref().unwrap();
            form.picker_focus(cx).focus(window, cx);
            assert!(form
                .runtime_select
                .read(cx)
                .focus_handle()
                .is_focused(window));
        });
    }

    #[test]
    #[cfg(unix)]
    fn terminal_key_fills_only_the_focused_empty_pane_and_otherwise_opens_a_tab() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let cwd = modal._temp.path().to_string_lossy().into_owned();
        modal.act(|root, window, cx| {
            root.close_start_chat_modal(window, cx);
            root.app_store.update(cx, |store, cx| {
                store.settings.default_working_dir = cwd.clone();
                cx.notify();
            });
            root.new_terminal_action(&NewTerminal, window, cx);
        });
        let mut first_tab = String::new();
        let mut empty = String::new();
        modal.act(|root, _, cx| {
            let layout = root.tabs.active_mut().unwrap();
            first_tab = layout.id.clone();
            empty = layout
                .split(&layout.focused_pane_id.clone(), SplitOrientation::Row)
                .unwrap();
            let input = layout.upsert_input().unwrap();
            runner_backend::ops::node::node_tab_upsert(root.core(cx), input).unwrap();
        });
        modal.act(|root, window, cx| {
            root.app_store.update(cx, |store, cx| {
                store.settings.default_working_dir = "/tmp".into();
                cx.notify();
            });
            assert_eq!(root.focused_empty_chat_pane(), Some(empty.clone()));
            root.new_terminal_action(&NewTerminal, window, cx);
            let layout = root.tabs.active().unwrap();
            assert_eq!(layout.id, first_tab);
            assert_eq!(layout.root.leaves().len(), 2);
            assert_eq!(layout.focused_pane_id, empty);
            let session = root
                .session_entry(layout.focused_session_id().unwrap(), cx)
                .unwrap();
            assert_eq!(session.cwd.as_deref(), Some(cwd.as_str()));
            assert_eq!(root.tabs.tabs().len(), 1);
            root.new_terminal_action(&NewTerminal, window, cx);
            assert_eq!(root.tabs.tabs().len(), 2);
        });
        modal.act(|root, window, cx| {
            root.tabs.activate(&first_tab);
            let layout = root.tabs.active_mut().unwrap();
            let extra = layout
                .split(&layout.focused_pane_id.clone(), SplitOrientation::Row)
                .unwrap();
            let occupied = layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.session_id.is_some())
                .unwrap()
                .id
                .clone();
            layout.focus_pane(&occupied);
            let input = layout.upsert_input().unwrap();
            runner_backend::ops::node::node_tab_upsert(root.core(cx), input).unwrap();
            assert_eq!(root.focused_empty_chat_pane(), None);
            root.new_terminal_action(&NewTerminal, window, cx);
            assert_eq!(root.tabs.tabs().len(), 3);
            assert!(root
                .tabs
                .tabs()
                .iter()
                .find(|tab| tab.id == first_tab)
                .unwrap()
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == extra)
                .unwrap()
                .session_id
                .is_none());
            root.set_route(AppRoute::Crews, cx);
            root.new_terminal_action(&NewTerminal, window, cx);
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.tabs().len(), 4);
            for session in root.app_store.read(cx).sessions.clone() {
                runner_backend::ops::session::session_close(root.core(cx), &session.session_id)
                    .unwrap();
            }
        });
    }

    #[test]
    #[cfg(unix)]
    fn terminal_key_from_a_mission_route_opens_a_chat_tab_in_the_active_project() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let cwd = modal._temp.path().to_string_lossy().into_owned();
        let mut project_id = String::new();
        modal.act(|root, window, cx| {
            root.close_start_chat_modal(window, cx);
            let project = runner_backend::ops::project::project_create(
                root.core(cx),
                "Terminal project".into(),
                cwd.clone(),
            )
            .unwrap();
            project_id = project.id.clone();
            let node = runner_backend::repo::node::ensure_project_node(
                &root.core(cx).db.get().unwrap(),
                &project.id,
            )
            .unwrap();
            let mut layout = PaneLayout::single(None, &[]);
            layout.id = "01M3VD00000000000000000005".into();
            layout.parent_id = Some(node.id);
            runner_backend::ops::node::node_tab_upsert(
                root.core(cx),
                layout.upsert_input().unwrap(),
            )
            .unwrap();
            root.app_store.update(cx, |store, cx| {
                store.projects = vec![project];
                cx.notify();
            });
            root.reload_tabs(cx).unwrap();
            root.set_route(AppRoute::Mission("test-mission".into()), cx);
        });
        modal.act(|root, window, cx| root.root_focus.focus(window, cx));
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-t"));
        modal.act(|root, _, cx| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.tabs().len(), 2);
            let session_id = root.tabs.active().unwrap().focused_session_id().unwrap();
            let session = root.session_entry(session_id, cx).unwrap();
            assert_eq!(session.project_id.as_deref(), Some(project_id.as_str()));
            assert_eq!(session.cwd.as_deref(), Some(cwd.as_str()));
            runner_backend::ops::session::session_close(root.core(cx), session_id).unwrap();
        });
    }

    #[test]
    fn confirm_does_nothing_when_disabled_starting_or_composing() {
        use gpui::EntityInputHandler;
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        modal.read(|modal, _| assert!(modal.error.is_none()));
        drop(modal);
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        modal.edit(|modal, cx| {
            modal.submitting = true;
            set_start_chat_controls_disabled(modal, true, cx);
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        modal.read(|modal, _| {
            assert!(modal.submitting);
            assert!(modal.error.is_none());
        });
        modal.edit(|modal, cx| {
            modal.submitting = false;
            set_start_chat_controls_disabled(modal, false, cx);
        });
        modal.act(|root, window, cx| {
            let title = root.start_chat_modal.as_ref().unwrap().title.clone();
            title.read(cx).focus_handle().focus(window, cx);
            title.update(cx, |title, cx| {
                title.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
            });
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        modal.read(|modal, cx| {
            assert!(modal.title.read(cx).is_composing());
            assert!(modal.error.is_none());
        });
    }

    #[test]
    fn mode_context_beats_global_tab_keys_and_arrows_keep_the_switch_focused() {
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![codex_role()],
            agents(),
            ChatMode::Runtime,
        );
        modal.act(|root, _, cx| {
            for id in ["01M3VD00000000000000000001", "01M3VD00000000000000000002"] {
                let mut layout = PaneLayout::single(None, &[]);
                layout.id = id.into();
                runner_backend::ops::node::node_tab_upsert(
                    root.core(cx),
                    layout.upsert_input().unwrap(),
                )
                .unwrap();
            }
            root.reload_tabs(cx).unwrap();
            root.tabs.activate("01M3VD00000000000000000001");
        });
        modal.act(|root, window, cx| {
            root.start_chat_modal
                .as_ref()
                .unwrap()
                .picker_focus(cx)
                .focus(window, cx)
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-2"));
        modal.act(|root, window, cx| {
            assert_eq!(
                root.tabs.active_tab_id(),
                Some("01M3VD00000000000000000001")
            );
            let form = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(form.mode, ChatMode::Role);
            assert!(form.role_select.read(cx).focus_handle().is_focused(window));
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-1"));
        modal.act(|root, window, cx| {
            let form = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(form.mode, ChatMode::Runtime);
            assert!(form
                .runtime_select
                .read(cx)
                .focus_handle()
                .is_focused(window));
        });
        let name_focus = modal.read(|form, cx| form.title.read(cx).focus_handle());
        modal.act(|_, window, cx| name_focus.focus(window, cx));
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-2"));
        modal.act(|root, window, cx| {
            let form = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(
                root.tabs.active_tab_id(),
                Some("01M3VD00000000000000000001")
            );
            assert_eq!(form.mode, ChatMode::Role);
            assert!(form.role_select.read(cx).focus_handle().is_focused(window));
        });
        modal.visual.simulate_keystrokes("enter");
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-1"));
        modal.act(|root, window, cx| {
            assert_eq!(
                root.tabs.active_tab_id(),
                Some("01M3VD00000000000000000001")
            );
            root.start_chat_modal
                .as_ref()
                .unwrap()
                .direct_mode_focus
                .focus(window, cx);
        });
        modal.visual.simulate_keystrokes("right");
        modal.act(|root, window, _cx| {
            let form = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(form.mode, ChatMode::Role);
            assert!(form.role_mode_focus.is_focused(window));
        });
        modal.visual.simulate_keystrokes("left space enter");
        modal.read(|form, _| assert_eq!(form.mode, ChatMode::Runtime));
        let bounds = modal.bounds("START_CHAT_ROLE_MODE").unwrap();
        modal
            .visual
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        modal.act(|root, window, _cx| {
            assert!(root
                .start_chat_modal
                .as_ref()
                .unwrap()
                .direct_mode_focus
                .is_focused(window))
        });
        modal.visual.simulate_keystrokes("escape");
        modal.act(|root, window, cx| root.root_focus.focus(window, cx));
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-2"));
        modal.act(|root, _, _| {
            assert_eq!(
                root.tabs.active_tab_id(),
                Some("01M3VD00000000000000000002")
            )
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-1"));
        modal.act(|root, _, _| {
            assert_eq!(
                root.tabs.active_tab_id(),
                Some("01M3VD00000000000000000001")
            )
        });
    }

    #[test]
    fn new_tab_from_outside_focus_closes_with_escape_and_name_enter_submits() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        modal.visual.simulate_keystrokes("escape");
        modal.act(|root, window, cx| {
            assert!(root.start_chat_modal.is_none());
            root.root_focus.focus(window, cx);
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-n"));
        modal.act(|root, window, cx| {
            let form = root.start_chat_modal.as_ref().unwrap();
            assert!(form.picker_focus(cx).is_focused(window));
        });
        modal.visual.simulate_keystrokes("escape");
        modal.act(|root, window, cx| {
            assert!(root.start_chat_modal.is_none());
            root.root_focus.focus(window, cx);
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-n"));
        modal.edit(|form, cx| seed_modal(form, vec![codex_role()], agents(), ChatMode::Role, cx));
        let name = modal.read(|form, cx| form.title.read(cx).focus_handle());
        modal.act(|_, window, cx| name.focus(window, cx));
        modal.visual.simulate_keystrokes("enter");
        modal.read(|form, _| assert!(form.error.is_some()));
    }

    #[test]
    #[cfg(unix)]
    fn new_tab_from_a_focused_terminal_pane_handles_escape_and_name_enter() {
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let cwd = modal._temp.path().to_string_lossy().into_owned();
        let mut session_id = String::new();
        modal.act(|root, window, cx| {
            root.close_start_chat_modal(window, cx);
            root.app_store.update(cx, |store, cx| {
                store.settings.default_working_dir = cwd;
                cx.notify();
            });
            root.new_terminal_tab(ProjectScope::Root, window, cx);
            session_id = root
                .tabs
                .active()
                .unwrap()
                .focused_session_id()
                .unwrap()
                .to_owned();
        });
        modal.act(|root, window, cx| {
            root.chat_transitions.clear();
            root.focus_active_terminal(window, cx);
            assert!(root
                .attached
                .get(&session_id)
                .unwrap()
                .terminal_focus
                .is_focused(window));
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-n"));
        modal.act(|root, window, cx| {
            assert!(root
                .start_chat_modal
                .as_ref()
                .unwrap()
                .picker_focus(cx)
                .is_focused(window));
        });
        modal.visual.simulate_keystrokes("escape");
        modal.act(|root, window, _cx| {
            assert!(root.start_chat_modal.is_none());
            assert!(root
                .attached
                .get(&session_id)
                .unwrap()
                .terminal_focus
                .is_focused(window));
        });
        modal
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-n"));
        modal.edit(|form, cx| seed_modal(form, vec![codex_role()], agents(), ChatMode::Role, cx));
        let name = modal.read(|form, cx| form.title.read(cx).focus_handle());
        modal.act(|_, window, cx| name.focus(window, cx));
        modal.visual.simulate_keystrokes("enter");
        modal.read(|form, _| assert!(form.error.is_some()));
        modal.act(|root, _, cx| {
            runner_backend::ops::session::session_close(root.core(cx), &session_id).unwrap();
        });
    }

    #[test]
    fn mouse_mode_change_from_the_initial_picker_keeps_modal_keys_live() {
        for mode in [ChatMode::Runtime, ChatMode::Role] {
            for confirm in [false, true] {
                let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), mode);
                let missing = modal
                    ._temp
                    .path()
                    .join("missing")
                    .to_string_lossy()
                    .into_owned();
                modal.edit(|form, cx| form.cwd.update(cx, |field, cx| field.set_text(missing, cx)));
                let selector = match mode {
                    ChatMode::Runtime => "START_CHAT_ROLE_MODE",
                    ChatMode::Role => "START_CHAT_DIRECT_MODE",
                };
                let bounds = modal.bounds(selector).unwrap();
                modal
                    .visual
                    .simulate_click(bounds.center(), gpui::Modifiers::default());
                modal.act(|root, window, cx| {
                    let form = root.start_chat_modal.as_ref().unwrap();
                    assert_ne!(form.mode, mode);
                    assert!(form.picker_focus(cx).is_focused(window));
                });
                if confirm {
                    modal.visual.simulate_keystrokes("tab");
                    modal
                        .visual
                        .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
                    modal.read(|form, _| assert!(form.error.is_some()));
                } else {
                    modal.visual.simulate_keystrokes("escape");
                    modal.act(|root, _, _| assert!(root.start_chat_modal.is_none()));
                }
            }
        }
    }

    #[test]
    fn mouse_mode_change_preserves_focus_in_a_persistent_text_field() {
        let mut modal = modal_harness(
            1200.,
            1000.,
            vec![codex_role()],
            agents(),
            ChatMode::Runtime,
        );
        let focus = modal.read(|form, cx| form.title.read(cx).focus_handle());
        modal.act(|_, window, cx| focus.focus(window, cx));
        let bounds = modal.bounds("START_CHAT_ROLE_MODE").unwrap();
        modal
            .visual
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        modal.act(|root, window, _| {
            assert_eq!(root.start_chat_modal.as_ref().unwrap().mode, ChatMode::Role);
            assert!(focus.is_focused(window));
        });
    }

    #[test]
    fn user_confirm_collision_cannot_shadow_chat_confirm_from_a_picker_or_text_field() {
        for text_field in [false, true] {
            let mut modal =
                modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
            let key = gpui::Keystroke::parse(&keymap::platform_default("cmd-enter")).unwrap();
            let overrides = keymap::KeymapOverrides::from([(
                "new-terminal".into(),
                keymap::combo_from_keystroke(&key),
            )]);
            modal.act(|root, window, cx| {
                keymap::install_bindings(cx, &overrides, false);
                if text_field {
                    root.start_chat_modal
                        .as_ref()
                        .unwrap()
                        .title
                        .read(cx)
                        .focus_handle()
                        .focus(window, cx);
                }
            });
            modal
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            modal.read(|form, _| assert!(form.error.is_some()));
        }
    }

    #[test]
    fn create_keys_are_inert_in_start_chat() {
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        let title = modal.read(|form, _| form.title.clone());
        for key in ["cmd-n", "cmd-t", "shift-cmd-m"] {
            modal
                .visual
                .simulate_keystrokes(&keymap::platform_default(key));
            modal.read(|form, _| assert_eq!(form.title, title));
            modal.act(|root, _, _| {
                assert!(root.start_mission_modal.is_none());
                assert!(root.tabs.tabs().is_empty());
            });
        }
    }

    #[test]
    fn tab_order_runs_from_the_picker_through_the_controls_and_resets() {
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        let order = |modal: &ModalHarness| {
            modal.read(|modal, cx| {
                let handles = start_chat_focus_order(modal, cx);
                let resets = |kind: ResetKind| modal.reset_focus[kind as usize].clone();
                let names = [
                    ("close", modal.close_focus.clone()),
                    ("direct", modal.direct_mode_focus.clone()),
                    ("role", modal.role_mode_focus.clone()),
                    ("role picker", modal.role_select.read(cx).focus_handle()),
                    ("runtime", modal.role_runtime_select.read(cx).focus_handle()),
                    ("model", modal.model.read(cx).focus_handle()),
                    ("effort", modal.effort_select.read(cx).focus_handle()),
                    ("speed", modal.speed_select.read(cx).focus_handle()),
                    ("reset runtime", resets(ResetKind::Runtime)),
                    ("reset model", resets(ResetKind::Model)),
                    ("reset effort", resets(ResetKind::Effort)),
                    ("reset speed", resets(ResetKind::Speed)),
                    ("agent picker", modal.runtime_select.read(cx).focus_handle()),
                    ("name", modal.title.read(cx).focus_handle()),
                    ("directory", modal.cwd.read(cx).focus_handle()),
                    ("browse", modal.browse_focus.clone()),
                    ("cancel", modal.cancel_focus.clone()),
                    ("start", modal.submit_focus.clone()),
                ];
                handles
                    .iter()
                    .map(|handle| {
                        names
                            .iter()
                            .find(|(_, named)| named == handle)
                            .map_or("?", |(name, _)| *name)
                    })
                    .collect::<Vec<_>>()
            })
        };
        let head = ["close", "role", "role picker"];
        let tail = ["name", "directory", "browse", "cancel", "start"];
        assert_eq!(
            order(&modal),
            [
                &head[..],
                &["runtime", "model", "effort", "speed"],
                &tail[..]
            ]
            .concat()
        );

        // A Reset joins the order after every control, in control order.
        modal.type_model("gpt-5.6-sol");
        modal.choose(StartChatSelection::Speed, "standard");
        assert_eq!(
            order(&modal),
            [
                &head[..],
                &[
                    "runtime",
                    "model",
                    "effort",
                    "speed",
                    "reset model",
                    "reset speed"
                ],
                &tail[..]
            ]
            .concat()
        );

        modal.act(|root, _, cx| root.set_start_chat_mode(ChatMode::Runtime, cx));
        assert_eq!(
            order(&modal),
            [
                &["close", "direct", "agent picker", "model", "effort"][..],
                &tail[..]
            ]
            .concat()
        );
    }

    #[test]
    fn tab_traversal_reaches_only_the_active_mode_segment() {
        let mut modal = modal_harness(1200., 1000., vec![codex_role()], agents(), ChatMode::Role);
        let order = modal.read(start_chat_focus_order);
        let close = order[0].clone();
        modal.act(|_, window, cx| close.focus(window, cx));
        for focus in order.iter().skip(1).chain(order.iter().take(1)) {
            modal.visual.simulate_keystrokes("tab");
            modal.act(|_, window, _| assert!(focus.is_focused(window)));
        }
        modal.visual.simulate_keystrokes("shift-tab");
        modal.act(|_, window, _| assert!(order.last().unwrap().is_focused(window)));
    }

    #[test]
    fn empty_states_draw_the_picker_alone() {
        // No roles yet.
        let mut modal = modal_harness(1200., 1000., Vec::new(), agents(), ChatMode::Role);
        assert!(modal.bounds("START_CHAT_CARD").is_some());
        assert!(modal.bounds("START_CHAT_SETUP").is_none());
        modal.read(|modal, _| assert!(!modal.can_submit()));
        drop(modal);

        // No enabled agents.
        let mut modal = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        assert!(modal.bounds("START_CHAT_CARD").is_some());
        assert!(modal.bounds("START_CHAT_SETUP").is_none());
        modal.read(|modal, cx| {
            assert!(!modal.can_submit());
            assert!(start_chat_focus_order(modal, cx)
                .iter()
                .all(|handle| *handle != modal.runtime_select.read(cx).focus_handle()));
        });
    }

    #[test]
    fn resets_are_inert_while_starting() {
        let mut modal = modal_harness(1200., 1000., vec![claude_role()], agents(), ChatMode::Role);
        modal.type_model("sonnet");
        modal.edit(|modal, cx| {
            modal.submitting = true;
            set_start_chat_controls_disabled(modal, true, cx);
        });
        modal.act(|root, window, cx| root.reset_start_chat_control(ResetKind::Model, window, cx));
        modal.read(|modal, cx| assert_eq!(modal.model.read(cx).text(), "sonnet"));
    }

    #[test]
    fn the_working_directory_hint_names_where_a_blank_field_starts() {
        let mut with_dir = test_role("dir", "claude-code");
        with_dir.working_dir = Some("/repo".into());
        let without = test_role("none", "claude-code");
        assert_eq!(
            working_dir_hint(ChatMode::Role, Some(&with_dir)),
            "Blank starts in the role's directory."
        );
        assert_eq!(
            working_dir_hint(ChatMode::Role, Some(&without)),
            "Blank starts in your default directory."
        );
        assert_eq!(
            working_dir_hint(ChatMode::Runtime, Some(&with_dir)),
            "Blank starts in your default directory."
        );
    }

    #[test]
    fn the_agent_note_says_which_role_values_do_not_carry_over() {
        assert_eq!(
            agent_note("Codex", "Claude Code", Some("opus[1m]"), Some("xhigh")),
            "Codex starts from its own model and effort. The role's opus[1m] and xhigh belong to Claude Code and don't carry over."
        );
        assert_eq!(
            agent_note("Codex", "Claude Code", None, Some("xhigh")),
            "Codex starts from its own model and effort. The role's xhigh belongs to Claude Code and doesn't carry over."
        );
        assert_eq!(
            agent_note("Codex", "Claude Code", None, None),
            "Codex starts from its own model and effort."
        );
    }

    #[test]
    fn card_columns_fit_the_width_the_modal_leaves_the_form() {
        let layout = CardLayout::new(510.);
        assert_eq!(layout.content, 484.);
        assert!(3. * layout.third() + 2. * COLUMN_GAP <= layout.content);
        assert_eq!(layout.model(false, false), layout.content);
        assert_eq!(
            layout.model(true, true) + EFFORT_COLUMN_WIDTH + SPEED_COLUMN_WIDTH + 2. * COLUMN_GAP,
            layout.content
        );
    }

    /// The modal keeps its width in the smallest window, scrolls, and lets
    /// nothing run past the card or the panel.
    #[test]
    fn the_modal_fits_a_640_by_480_window_without_clipping_sideways() {
        let long = Role {
            display_name: "A role whose display name is far too long to fit a picker".repeat(3),
            handle: "a-handle-that-goes-on-and-on-and-on-and-on-and-on-and-on".into(),
            model: Some("a-model-name-that-is-longer-than-its-column-can-hold".into()),
            ..codex_role()
        };
        let mut modal = modal_harness(640., 480., vec![long], agents(), ChatMode::Role);
        let fits = |modal: &mut ModalHarness, fields: &[&str]| {
            let panel = modal.bounds("MODAL_PANEL").unwrap();
            assert_eq!(panel.size.width, px(560.));
            assert!(
                panel.left() >= px(0.) && panel.right() <= px(640.),
                "{panel:?}"
            );
            assert!(panel.size.height <= px(480. * 0.85 + 1.), "{panel:?}");
            let card = modal.bounds("START_CHAT_CARD").unwrap();
            assert!(
                card.left() >= panel.left() + px(24.),
                "{card:?} in {panel:?}"
            );
            assert!(
                card.right() <= panel.right() - px(24.),
                "{card:?} in {panel:?}"
            );
            for field in fields {
                let bounds = modal.bounds(&format!("START_CHAT_FIELD {field}")).unwrap();
                assert!(
                    bounds.left() >= card.left() && bounds.right() <= card.right(),
                    "{field}: {bounds:?} in {card:?}"
                );
            }
        };
        fits(&mut modal, &["Runtime", "Model", "Effort", "Speed"]);
        let picker = modal.bounds("STYLED_SELECT_TRIGGER").unwrap();
        let card = modal.bounds("START_CHAT_CARD").unwrap();
        assert!(
            picker.left() >= card.left() && picker.right() <= card.right(),
            "a long name truncates inside the picker: {picker:?} in {card:?}"
        );
        assert!(
            modal.read(|modal, _| modal.scroll_handle.max_offset().y) > px(0.),
            "the tall form scrolls"
        );

        // Overridden: notes and Resets sit inside their columns too.
        modal.type_model("a-model-name-that-is-far-too-long-for-its-narrow-column-to-hold");
        modal.choose(StartChatSelection::Speed, "standard");
        fits(&mut modal, &["Runtime", "Model", "Effort", "Speed"]);
        for reset in [ResetKind::Model, ResetKind::Speed] {
            let reset = modal
                .bounds(&format!("START_CHAT_RESET {reset:?}"))
                .unwrap();
            let card = modal.bounds("START_CHAT_CARD").unwrap();
            assert!(reset.right() <= card.right(), "{reset:?} in {card:?}");
        }

        modal.act(|root, _, cx| root.set_start_chat_mode(ChatMode::Runtime, cx));
        modal.choose(StartChatSelection::Runtime, "codex");
        fits(&mut modal, &["Model", "Effort", "Speed"]);
    }

    #[test]
    fn new_terminal_targets_an_available_mission_drawer_then_the_active_tab_kind() {
        let mission = AppRoute::Mission("mission".into());
        let mut terminals = PaneLayout::single(Some("shell"), &[]);
        let pane = terminals.split("p1", SplitOrientation::Row).unwrap();
        terminals.assign_session(&pane, "shell-2").unwrap();
        let terminal_tab =
            super::super::chat::tab_is_terminal(&terminals, |_| Some(Runtime::Shell));
        assert!(terminal_tab);
        for terminal in [false, terminal_tab] {
            assert_eq!(
                new_terminal_target(&mission, true, terminal),
                NewTerminalTarget::MissionDrawer
            );
            for route in [
                AppRoute::Chat,
                AppRoute::Roles,
                AppRoute::RoleDetail("role".into()),
                AppRoute::Crews,
                AppRoute::CrewEditor("crew".into()),
                AppRoute::Settings,
                AppRoute::ArchivedChat,
                mission.clone(),
            ] {
                assert_eq!(
                    new_terminal_target(&route, false, terminal),
                    if terminal {
                        NewTerminalTarget::Tab
                    } else {
                        NewTerminalTarget::ChatDrawer
                    },
                    "{route:?}, terminal={terminal}"
                );
            }
        }
    }

    #[test]
    fn new_terminal_prefers_the_focused_empty_pane_then_any_empty_pane() {
        let mut layout = PaneLayout::single(Some("shell"), &[]);
        assert_eq!(new_terminal_empty_pane(&layout), None);

        let first = layout.split("p1", SplitOrientation::Row).unwrap();
        let second = layout.split(&first, SplitOrientation::Column).unwrap();
        assert_eq!(new_terminal_empty_pane(&layout), Some(second.clone()));

        layout.focus_session("shell");
        assert_eq!(new_terminal_empty_pane(&layout), Some(first.clone()));

        layout.assign_session(&first, "shell-2").unwrap();
        assert_eq!(new_terminal_empty_pane(&layout), Some(second.clone()));

        layout.assign_session(&second, "shell-3").unwrap();
        assert_eq!(new_terminal_empty_pane(&layout), None);
    }

    fn runtime(name: &str, efforts: &[&str]) -> RuntimeCatalogEntry {
        RuntimeCatalogEntry {
            name: Runtime::parse(name).unwrap(),
            capabilities: runner_backend::ops::runtime::RuntimeCatalogEntry::for_runtime(
                Runtime::parse(name).unwrap(),
            )
            .map(|entry| entry.capabilities)
            .unwrap_or_default(),
            display_name: name.into(),
            command: name.into(),
            native_fork: matches!(name, "codex" | "claude-code" | "pi"),
            description: name.into(),
            install_url: String::new(),
            default_enabled: true,
            available: true,
            default_model: None,
            default_effort: None,
            models: Vec::new(),
            efforts: efforts
                .iter()
                .map(|value| RuntimeCatalogOption {
                    value: (*value).into(),
                    label: (*value).into(),
                    description: None,
                    supported_efforts: None,
                })
                .collect(),
        }
    }

    #[test]
    fn new_chat_only_saves_names_the_user_edited() {
        let mut cx = gpui::TestAppContext::single();
        let title = cx.new(|cx| TextField::new(cx.focus_handle(), "", "Codex", false));
        title.update(&mut cx, |input, cx| {
            assert_eq!(input.text(), "");
            assert!(!input.edited());
            assert_eq!(user_chat_title(input), None);
            input.set_placeholder("@coder", cx);
            assert_eq!(user_chat_title(input), None);
            input.set_text("  Cars  ", cx);
            assert_eq!(user_chat_title(input).as_deref(), Some("Cars"));
            input.set_text("Codex", cx);
            assert_eq!(user_chat_title(input).as_deref(), Some("Codex"));
            input.set_text("", cx);
            assert_eq!(user_chat_title(input), None);
            input.set_text("   ", cx);
            assert_eq!(user_chat_title(input), None);
        });
    }

    #[test]
    fn title_selection_changes_preserve_blank_and_user_edited_text() {
        let mut cx = gpui::TestAppContext::single();
        let title = cx.new(|cx| {
            TextField::new(
                cx.focus_handle(),
                "",
                default_title_for_role("coder"),
                false,
            )
        });
        cx.update(|cx| {
            update_auto_title(&title, default_title_for_role("reviewer"), cx);
            assert_eq!(title.read(cx).text(), "");
            assert!(!title.read(cx).edited());
            assert_eq!(user_chat_title(title.read(cx)), None);

            title.update(cx, |input, cx| input.set_text("my chat", cx));
            update_auto_title(&title, default_title_for_runtime("Codex"), cx);
            assert_eq!(user_chat_title(title.read(cx)).as_deref(), Some("my chat"));

            title.update(cx, |input, cx| input.set_text("", cx));
            update_auto_title(&title, default_title_for_role("coder"), cx);
            assert_eq!(title.read(cx).text(), "");
            assert_eq!(user_chat_title(title.read(cx)), None);
            update_auto_title(&title, default_title_for_runtime("Claude Code"), cx);
            assert_eq!(title.read(cx).text(), "");
            assert_eq!(user_chat_title(title.read(cx)), None);
        });
    }

    #[test]
    fn effort_options_follow_the_selected_runtime_catalog() {
        let runtimes = [runtime("codex", &["", "low", "max"]), runtime("trae", &[])];
        assert_eq!(
            effort_options_for_runtime(&runtimes, "codex")
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["", "low", "max"]
        );
        assert!(effort_options_for_runtime(&runtimes, "trae").is_empty());
        assert!(effort_options_for_runtime(&runtimes, "missing").is_empty());
    }

    #[test]
    fn a_shell_started_from_a_session_prefers_its_live_cwd_then_its_spawn_cwd() {
        let live = Some(PathBuf::from("/repo/crates/app"));
        assert_eq!(
            shell_start_cwd(live.clone(), Some("/repo")).as_deref(),
            Some("/repo/crates/app")
        );
        assert_eq!(
            shell_start_cwd(live, None).as_deref(),
            Some("/repo/crates/app")
        );
        assert_eq!(
            shell_start_cwd(None, Some("/repo")).as_deref(),
            Some("/repo")
        );
        assert_eq!(shell_start_cwd(None, None), None);
        assert_eq!(
            terminal_working_dir(
                shell_start_cwd(None, None).as_deref(),
                Some("/project"),
                "/settings",
                Some("/home")
            )
            .as_deref(),
            Some("/project")
        );
    }

    #[test]
    fn terminal_cwd_prefers_the_focused_sibling_then_project_settings_and_home() {
        assert_eq!(
            terminal_working_dir(
                Some("/sibling"),
                Some("/project"),
                "/settings",
                Some("/home")
            )
            .as_deref(),
            Some("/sibling")
        );
        assert_eq!(
            terminal_working_dir(None, Some("/project"), "/settings", Some("/home")).as_deref(),
            Some("/project")
        );
        assert_eq!(
            terminal_working_dir(None, None, "/settings", Some("/home")).as_deref(),
            Some("/settings")
        );
        assert_eq!(
            terminal_working_dir(None, None, " ", Some("/home")).as_deref(),
            Some("/home")
        );
    }

    #[test]
    fn mode_preference_round_trips_and_invalid_values_fall_back_to_direct() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runtime);

        write_start_chat_mode(temp.path(), ChatMode::Role).unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Role);
        assert_eq!(
            fs::read_to_string(start_chat_mode_path(temp.path())).unwrap(),
            "role"
        );

        fs::write(start_chat_mode_path(temp.path()), "runner").unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Role);

        write_start_chat_mode(temp.path(), ChatMode::Runtime).unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runtime);

        fs::write(start_chat_mode_path(temp.path()), "unexpected").unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runtime);
    }

    #[test]
    fn project_scope_seeds_cwd_before_role_and_settings_defaults() {
        let project = runner_backend::repo::project::ProjectRow {
            id: "project-1".into(),
            name: "Runner".into(),
            cwd: "/project".into(),
            position: 0,
            created_at: "now".into(),
        };
        let (scope, seeded_cwd) = project_start_scope(Some(&project));
        assert_eq!(scope, ProjectScope::Project("project-1".into()));
        assert_eq!(seeded_cwd, "/project");
        assert_eq!(
            effective_working_dir(&seeded_cwd, true, "/settings"),
            Some("/project".into())
        );

        let (scope, seeded_cwd) = project_start_scope(None);
        assert_eq!(scope, ProjectScope::Root);
        assert!(seeded_cwd.is_empty());
        assert_eq!(
            effective_working_dir(&seeded_cwd, false, "/settings"),
            Some("/settings".into())
        );
    }
}
