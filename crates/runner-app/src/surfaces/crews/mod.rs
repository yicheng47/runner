pub(crate) use logic::{crew_picture, slot_setup, SlotSetup};
pub(crate) use slots::{lead_badge, slot_setup_line};
mod add_slot;
mod create;
mod editor;
mod editor_sections;
mod list;
mod logic;
mod overlays;
mod popup;
mod slots;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, rems, Bounds, Context, Entity, FocusHandle, Pixels, ScrollHandle, Subscription, Window,
};
use runner_app::ui::{ContextMenu, ModelField, Scrollbar, SearchInput, StyledSelect, TextField};
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::model::{Crew, SlotWithRole};
use runner_core::protocol::role::RoleWithActivity;
use runner_core::protocol::runtime::RuntimeCatalogEntry;

use crate::list_controls::ListControls;
use crate::*;

const FORM_WIDTH: f32 = 576.;
const FIELD_WIDTH: f32 = 528.;

#[derive(Clone)]
struct SlotDrag {
    slot_id: String,
    label: String,
}

impl Render for SlotDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(rems(220. / 16.))
            .px_3()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(theme::accent())
            .bg(theme::panel())
            .shadow_lg()
            .font_family(theme::UI_MONOSPACE_FONT)
            .text_size(theme::text_body())
            .text_color(theme::text())
            .child(self.label.clone())
    }
}

#[derive(Default)]
struct CrewEditorState {
    crew_id: String,
    crew: Option<Crew>,
    slots: Vec<SlotWithRole>,
    loaded: bool,
    loading: bool,
    error: Option<String>,
    /// Edit in place: the name and conventions, saved together.
    edit: Option<CrewEditForm>,
    /// The conventions card shows the whole text instead of its first lines.
    conventions_expanded: bool,
    /// The conventions editor shows the rendered draft.
    conventions_preview: bool,
    /// The missions card lists every mission instead of the latest four.
    missions_expanded: bool,
    popup: Option<SlotPopup>,
    reordering: bool,
    dragged_slot_id: Option<String>,
    drop_target: Option<usize>,
}

struct CrewEditForm {
    name: Entity<TextField>,
    conventions: Entity<TextField>,
    mode_focus: [FocusHandle; 2],
    action_focus: [FocusHandle; 2],
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

/// The popover beside a clicked slot row.
struct SlotPopup {
    slot_id: String,
    /// The row's bounds, which its canvas refreshes every frame.
    anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    open_role_focus: FocusHandle,
    remove_focus: FocusHandle,
    edit: Option<SlotOverrideForm>,
}

/// Edit overrides saved to the slot.
struct SlotOverrideForm {
    runtimes: Vec<RuntimeCatalogEntry>,
    /// The runtime override; `None` runs the role's runtime.
    runtime: Option<String>,
    runtime_select: Entity<StyledSelect>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    /// The effort override; empty inherits.
    effort: String,
    effort_select: Entity<StyledSelect>,
    speed: Option<runner_core::protocol::model::CodexSpeed>,
    speed_select: Entity<StyledSelect>,
    /// Reset for runtime, model, effort and Speed.
    reset_focus: [FocusHandle; 4],
    saving: bool,
    error: Option<crate::i18n::Text>,
    _subscriptions: Vec<Subscription>,
}

struct CreateCrewForm {
    name: Entity<TextField>,
    conventions: Entity<TextField>,
    mode_focus: [FocusHandle; 2],
    action_focus: [FocusHandle; 2],
    return_route: AppRoute,
    submitting: bool,
    error: Option<crate::i18n::Text>,
    _subscriptions: Vec<Subscription>,
}

struct AddSlotForm {
    crew_id: String,
    crew_name: String,
    existing_handles: HashSet<String>,
    roles: Vec<RoleWithActivity>,
    runtimes: Vec<RuntimeCatalogEntry>,
    query: Entity<TextField>,
    last_synced_query: String,
    selected_role_id: Option<String>,
    slot_handle: Entity<TextField>,
    runtime_override: String,
    model_override: Entity<TextField>,
    model_field: Entity<ModelField>,
    runtime_select: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    slot_handle_hint_focus: FocusHandle,
    runtime_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    slot_handle_empty: bool,
    slot_handle_error: Option<crate::i18n::Text>,
    loading: bool,
    submitting: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
enum CrewMenuAction {
    Open(String),
    Delete { id: String, name: String },
}

struct CrewDeleteConfirm {
    id: String,
    name: String,
}

struct SlotRemoveConfirm {
    slot: SlotWithRole,
}

#[derive(Clone)]
pub(crate) enum CrewListRow {
    Local(CrewListItem),
    Remote {
        crew: runner_core::companion::CatalogCrew,
        role_handles: Vec<Option<String>>,
        runtimes: Vec<(String, usize)>,
        target: crate::surfaces::front_page::FrontPageTarget,
        epoch: u64,
    },
}

impl CrewListRow {
    fn id(&self) -> &str {
        match self {
            Self::Local(item) => &item.crew.id,
            Self::Remote { crew, .. } => &crew.id,
        }
    }
    fn name(&self) -> &str {
        match self {
            Self::Local(item) => &item.crew.name,
            Self::Remote { crew, .. } => &crew.name,
        }
    }
    fn handles(&self) -> Vec<gpui::SharedString> {
        match self {
            Self::Local(item) => item
                .members
                .iter()
                .map(|slot| slot.slot_handle.clone().into())
                .collect(),
            Self::Remote { crew, .. } => crew
                .slots
                .iter()
                .map(|slot| slot.handle.clone().into())
                .collect(),
        }
    }
    fn local(&self) -> Option<&CrewListItem> {
        match self {
            Self::Local(item) => Some(item),
            Self::Remote { .. } => None,
        }
    }
    fn summary(&self) -> String {
        match self {
            Self::Local(item) => logic::crew_summary(
                item.members.len(),
                item.members
                    .iter()
                    .find(|slot| slot.lead)
                    .map(|slot| slot.slot_handle.as_str()),
            ),
            Self::Remote {
                crew, role_handles, ..
            } => crew
                .slots
                .iter()
                .zip(role_handles)
                .map(|(slot, role_handle)| {
                    let lead = if slot.lead {
                        format!(" · {}", crate::i18n::text("ui.lead"))
                    } else {
                        String::new()
                    };
                    let role = role_handle
                        .as_ref()
                        .map(|handle| format!(": @{handle}"))
                        .unwrap_or_default();
                    format!("@{}{}{}", slot.handle, role, lead)
                })
                .collect::<Vec<_>>()
                .join("; "),
        }
    }
}

pub(crate) struct CrewSurfaces {
    list: ListControls<CrewListRow>,
    /// The list row under the pointer, which trades its play icon for a button.
    hovered_row: Option<String>,
    search: Entity<SearchInput>,
    page_size: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    editor: CrewEditorState,
    create: Option<CreateCrewForm>,
    add_slot: Option<AddSlotForm>,
    pub(crate) context_menu: Option<Entity<ContextMenu>>,
    delete_confirm: Option<CrewDeleteConfirm>,
    delete_busy: bool,
    slot_remove_confirm: Option<SlotRemoveConfirm>,
    slot_remove_busy: bool,
}

impl CrewSurfaces {
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
            "crew-page-size",
            page_size,
            Rc::new(move |size, _, cx| {
                size_root.update(cx, |this, cx| match remote_epoch {
                    None => this.set_local_crew_page_size(size, cx),
                    Some(epoch)
                        if this.remote_window.is_some() && this.client_ui.epoch == epoch =>
                    {
                        this.set_crew_page_size(size, cx)
                    }
                    Some(_) => {}
                });
            }),
            cx,
        );
        let search = cx.new(move |search_cx| {
            SearchInput::new(
                "",
                crate::i18n::message("ui.search_crews"),
                crate::i18n::message("ui.search_crews_slots_and_roles"),
                Rc::new(move |query, cx| {
                    root.update(cx, |this, cx| match remote_epoch {
                        None => this.set_local_crew_query(query, cx),
                        Some(epoch)
                            if this.remote_window.is_some() && this.client_ui.epoch == epoch =>
                        {
                            this.set_crew_query(query, cx)
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
            hovered_row: None,
            search,
            page_size: size_select,
            scroll,
            scrollbar,
            editor: CrewEditorState::default(),
            create: None,
            add_slot: None,
            context_menu: None,
            delete_confirm: None,
            delete_busy: false,
            slot_remove_confirm: None,
            slot_remove_busy: false,
        }
    }
}
