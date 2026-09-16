mod create;
mod delete;
mod detail;
mod edit;
mod forms;
mod list;
mod logic;
mod menu;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{Context, Entity, FocusHandle, ScrollHandle, Subscription};
use runner_app::ui::{
    ContextMenu, ModelField, RuntimeSelect, Scrollbar, SearchInput, StyledSelect, TextField,
};
use runner_backend::model::Role;
use runner_backend::ops::role::{RoleActivity, RoleWithActivity};
use runner_backend::ops::runtime::RuntimeCatalogEntry;
use runner_backend::ops::slot::CrewMembership;
use runner_backend::router::runtime::PermissionMode;

use crate::list_controls::ListControls;
use crate::*;

const FORM_WIDTH: f32 = 576.;
const FIELD_WIDTH: f32 = 528.;

#[derive(Default)]
struct RoleDetailState {
    handle: String,
    role: Option<Role>,
    activity: Option<RoleActivity>,
    crews: Vec<CrewMembership>,
    loaded: bool,
    loading: bool,
    error: Option<String>,
}

#[derive(Clone)]
enum RoleMenuAction {
    Open(String),
    Delete { id: String, handle: String },
}

struct RoleDeleteConfirm {
    id: String,
    handle: String,
}

struct CreateRoleForm {
    runtimes: Vec<RuntimeCatalogEntry>,
    runtime: String,
    permission_mode: PermissionMode,
    handle: Entity<TextField>,
    display_name: Entity<TextField>,
    command: Entity<TextField>,
    args: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    working_dir: Entity<TextField>,
    system_prompt: Entity<TextField>,
    runtime_select: Entity<RuntimeSelect>,
    permission_select: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    browse_focus: FocusHandle,
    args_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    permission_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    handle_empty: bool,
    handle_error: Option<&'static str>,
    display_name_valid: bool,
    submitting: bool,
    agents_checking: bool,
    agents_error: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct RoleEditForm {
    role: Role,
    slot: Option<runner_backend::model::SlotWithRole>,
    runtimes: Vec<RuntimeCatalogEntry>,
    runtime: String,
    runtime_pinned: bool,
    permission_mode: PermissionMode,
    display_name: Entity<TextField>,
    command: Entity<TextField>,
    args: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    effort: String,
    effort_select: Entity<StyledSelect>,
    permission_select: Entity<StyledSelect>,
    runtime_select: Entity<RuntimeSelect>,
    working_dir: Entity<TextField>,
    system_prompt: Entity<TextField>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    browse_focus: FocusHandle,
    runtime_hint_focus: FocusHandle,
    args_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    effort_hint_focus: FocusHandle,
    permission_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    display_name_valid: bool,
    submitting: bool,
    agents_checking: bool,
    agents_error: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct RoleSurfaces {
    list: ListControls<RoleWithActivity>,
    search: Entity<SearchInput>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    detail: RoleDetailState,
    create: Option<CreateRoleForm>,
    edit: Option<RoleEditForm>,
    context_menu: Option<Entity<ContextMenu>>,
    delete_confirm: Option<RoleDeleteConfirm>,
    delete_busy: bool,
    chat_pending: Option<String>,
}

impl RoleSurfaces {
    pub(crate) fn new(root: Entity<NativeRoot>, cx: &mut Context<NativeRoot>) -> Self {
        let search_root = root;
        let search = cx.new(move |search_cx| {
            SearchInput::new(
                "",
                "Search roles",
                "Search roles…",
                Rc::new(move |query, cx| {
                    search_root.update(cx, |this, cx| this.set_role_query(query, cx));
                }),
                search_cx,
            )
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        Self {
            list: ListControls::default(),
            search,
            scroll,
            scrollbar,
            detail: RoleDetailState::default(),
            create: None,
            edit: None,
            context_menu: None,
            delete_confirm: None,
            delete_busy: false,
            chat_pending: None,
        }
    }
}

struct RoleEditResolution {
    runtime: String,
    runtime_pinned: bool,
    command: String,
    model: String,
    effort: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeLayerResolution {
    runtime: String,
    runtime_pinned: bool,
    model: Option<String>,
    effort: Option<String>,
}
