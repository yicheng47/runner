mod create;
mod delete;
mod detail;
mod edit;
mod forms;
mod list;
pub(crate) mod logic;
mod menu;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{Context, Entity, FocusHandle, ScrollHandle, Subscription};
use runner_app::ui::{ContextMenu, ModelField, Scrollbar, SearchInput, StyledSelect, TextField};
use runner_core::protocol::model::Role;
use runner_core::protocol::permissions::PermissionMode;
use runner_core::protocol::role::{RoleActivity, RoleWithActivity};
use runner_core::protocol::runtime::RuntimeCatalogEntry;
use runner_core::protocol::slot::CrewMembership;

use crate::list_controls::ListControls;
use crate::*;

/// The role page's left column, which holds the profile, setup and crews.
const ROLE_COLUMN_WIDTH: f32 = crate::surfaces::profile_page::PROFILE_COLUMN_WIDTH;

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
    Edit(Box<Role>),
    Delete { id: String, handle: String },
}

struct RoleDeleteConfirm {
    id: String,
    handle: String,
}

struct CreateRoleForm {
    fields: RoleFormFields,
    return_route: AppRoute,
    handle: Entity<TextField>,
    handle_empty: bool,
    handle_error: Option<&'static str>,
}

impl CreateRoleForm {
    fn avatar_seed(&self, cx: &gpui::App) -> String {
        let handle = self.handle.read(cx).text();
        if handle.is_empty() {
            format!("new-role-{}", self.handle.entity_id())
        } else {
            handle.to_owned()
        }
    }
}

struct RoleEditForm {
    role: Role,
    fields: RoleFormFields,
    permission_mode: PermissionMode,
}

struct RoleFormFields {
    runtimes: Vec<RuntimeCatalogEntry>,
    runtime: String,
    display_name: Entity<TextField>,
    command: Entity<TextField>,
    args: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    effort: String,
    effort_select: Entity<StyledSelect>,
    speed: String,
    speed_select: Entity<StyledSelect>,
    runtime_select: Entity<StyledSelect>,
    working_dir: Entity<TextField>,
    system_prompt: Entity<TextField>,
    prompt_mode_focus: [FocusHandle; 2],
    browse_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    display_name_valid: bool,
    submitting: bool,
    agents_checking: bool,
    agents_error: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl std::ops::Deref for CreateRoleForm {
    type Target = RoleFormFields;
    fn deref(&self) -> &Self::Target {
        &self.fields
    }
}

impl std::ops::DerefMut for CreateRoleForm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.fields
    }
}

impl std::ops::Deref for RoleEditForm {
    type Target = RoleFormFields;
    fn deref(&self) -> &Self::Target {
        &self.fields
    }
}

impl std::ops::DerefMut for RoleEditForm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.fields
    }
}

#[derive(Clone)]
pub(crate) enum RoleListRow {
    Local(RoleWithActivity),
    Remote {
        role: runner_core::companion::CatalogRole,
        crew_count: usize,
        target: crate::surfaces::front_page::FrontPageTarget,
        epoch: u64,
    },
}

struct RoleListDisplay<'a> {
    id: &'a str,
    handle: &'a str,
    display_name: &'a str,
    runtime: &'a str,
    model: Option<&'a str>,
    effort: Option<&'a str>,
}

impl RoleListRow {
    fn display(&self) -> RoleListDisplay<'_> {
        match self {
            Self::Local(item) => RoleListDisplay {
                id: &item.role.id,
                handle: &item.role.handle,
                display_name: &item.role.display_name,
                runtime: &item.role.runtime,
                model: item.role.model.as_deref(),
                effort: item.role.effort.as_deref(),
            },
            Self::Remote { role, .. } => RoleListDisplay {
                id: &role.id,
                handle: &role.handle,
                display_name: &role.name,
                runtime: &role.runtime,
                model: role.model.as_deref(),
                effort: role.effort.as_deref(),
            },
        }
    }

    fn local(&self) -> Option<&RoleWithActivity> {
        match self {
            Self::Local(item) => Some(item),
            Self::Remote { .. } => None,
        }
    }
}

pub(crate) struct RoleSurfaces {
    list: ListControls<RoleListRow>,
    search: Entity<SearchInput>,
    page_size: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    detail: RoleDetailState,
    create: Option<CreateRoleForm>,
    edit: Option<RoleEditForm>,
    context_menu: Option<Entity<ContextMenu>>,
    delete_confirm: Option<RoleDeleteConfirm>,
    delete_busy: bool,
    chat_pending: Option<String>,
    /// The role page shows the whole prompt instead of its first lines.
    prompt_expanded: bool,
    /// The in-place editor's prompt card shows the rendered draft.
    prompt_preview: bool,
    /// The list row under the pointer, which trades its Chat icon for a button.
    hovered_row: Option<String>,
}

impl RoleSurfaces {
    pub(crate) fn set_library_load(&mut self, error: Option<String>) {
        self.list.loading = error.is_none();
        self.list.error = error;
    }
    pub(crate) fn invalidate_library(&mut self) {
        self.list.items.clear();
        self.list.filtered_count = 0;
        self.list.total_count = 0;
        self.list.loaded = false;
        self.list.loading = true;
        self.list.error = None;
    }
    pub(crate) fn new(
        root: Entity<NativeRoot>,
        page_size: usize,
        cx: &mut Context<NativeRoot>,
    ) -> Self {
        Self::new_for_window(root, page_size, None, cx)
    }

    pub(crate) fn new_remote(
        root: Entity<NativeRoot>,
        page_size: usize,
        epoch: u64,
        cx: &mut Context<NativeRoot>,
    ) -> Self {
        Self::new_for_window(root, page_size, Some(epoch), cx)
    }

    fn new_for_window(
        root: Entity<NativeRoot>,
        page_size: usize,
        remote_epoch: Option<u64>,
        cx: &mut Context<NativeRoot>,
    ) -> Self {
        let size_root = root.clone();
        let size_select = runner_app::ui::list::page_size_select(
            "role-page-size",
            page_size,
            Rc::new(move |size, _, cx| {
                size_root.update(cx, |this, cx| match remote_epoch {
                    None => this.set_local_role_page_size(size, cx),
                    Some(epoch)
                        if this.remote_window.is_some() && this.client_ui.epoch == epoch =>
                    {
                        this.set_role_page_size(size, cx)
                    }
                    Some(_) => {}
                });
            }),
            cx,
        );
        let search_root = root;
        let search = cx.new(move |search_cx| {
            SearchInput::new(
                "",
                crate::i18n::message("ui.roles_search_label"),
                crate::i18n::message("ui.roles_search_placeholder"),
                Rc::new(move |query, cx| {
                    search_root.update(cx, |this, cx| match remote_epoch {
                        None => this.set_local_role_query(query, cx),
                        Some(epoch)
                            if this.remote_window.is_some() && this.client_ui.epoch == epoch =>
                        {
                            this.set_role_query(query, cx)
                        }
                        Some(_) => {}
                    });
                }),
                search_cx,
            )
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let mut list = ListControls::default();
        list.set_page_size(page_size);
        Self {
            list,
            search,
            page_size: size_select,
            scroll,
            scrollbar,
            detail: RoleDetailState::default(),
            create: None,
            edit: None,
            context_menu: None,
            delete_confirm: None,
            delete_busy: false,
            chat_pending: None,
            prompt_expanded: false,
            prompt_preview: false,
            hovered_row: None,
        }
    }
}

struct RoleEditResolution {
    runtime: String,
    command: String,
    model: String,
    effort: String,
    speed: String,
}
